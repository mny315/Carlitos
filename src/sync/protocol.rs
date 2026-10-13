use super::{MAX_RECORDS, RateRecord, Record, valid_hex};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

const LIMIT: usize = 4 * 1024 * 1024;
const CHUNK: usize = 32 * 1024;
const PATTERN: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";

#[derive(Serialize, Deserialize)]
pub(super) struct Hello {
    pub version: u32,
    pub from: String,
    pub to: String,
    pub invitation: String,
}
impl Hello {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1 && valid_hex(&self.from, 16) && valid_hex(&self.to, 16),
            "Invalid sync handshake"
        );
        ensure!(self.from != self.to, "Cannot pair a device with itself");
        ensure!(
            self.invitation.is_empty() || valid_hex(&self.invitation, 16),
            "Invalid invitation"
        );
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
pub(super) struct Exchange {
    pub name: String,
    pub port: u16,
    pub records: Vec<Record>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rates: Vec<RateRecord>,
}
impl Exchange {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.port != 0, "Invalid listening port");
        ensure!(
            !self.name.is_empty()
                && self.name.len() <= 128
                && !self.name.chars().any(char::is_control),
            "Invalid device name"
        );
        ensure!(self.records.len() <= MAX_RECORDS, "Too many sync records");
        for record in &self.records {
            record.validate()?;
        }
        ensure!(self.rates.len() <= MAX_RECORDS, "Too many book speeds");
        for rate in &self.rates {
            rate.validate()?;
        }
        Ok(())
    }
}

pub(super) struct Channel {
    stream: TcpStream,
    noise: snow::TransportState,
    deadline: Instant,
    compact: bool,
    peer_offer: Vec<u8>,
}
pub(super) fn configure(stream: &TcpStream) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.set_nodelay(true)?;
    Ok(())
}
pub(super) fn read_hello(stream: &mut TcpStream) -> Result<(Hello, Vec<u8>)> {
    configure(stream)?;
    let bytes = read_frame(stream, 512)?;
    let hello: Hello = serde_json::from_slice(&bytes)?;
    hello.validate()?;
    Ok((hello, bytes))
}
impl Channel {
    pub fn client(
        mut stream: TcpStream,
        key: &[u8; 32],
        hello: &Hello,
        exchange: &Exchange,
    ) -> Result<Self> {
        hello.validate()?;
        configure(&stream)?;
        let bytes = serde_json::to_vec(hello)?;
        write_frame(&mut stream, &bytes)?;
        Self::handshake(stream, key, &bytes, true, exchange)
    }
    pub fn server(
        stream: TcpStream,
        key: &[u8; 32],
        hello: &[u8],
        exchange: &Exchange,
    ) -> Result<Self> {
        Self::handshake(stream, key, hello, false, exchange)
    }
    fn handshake(
        mut stream: TcpStream,
        key: &[u8; 32],
        hello: &[u8],
        initiator: bool,
        exchange: &Exchange,
    ) -> Result<Self> {
        let builder = snow::Builder::new(PATTERN.parse()?)
            .psk(0, key)?
            .prologue(hello)?;
        let mut noise = if initiator {
            builder.build_initiator()?
        } else {
            builder.build_responder()?
        };
        let mut buffer = vec![0; 1024];
        let offer = super::compact::offer(exchange)?;
        let peer_offer;
        if initiator {
            let count = noise.write_message(&offer, &mut buffer)?;
            write_frame(&mut stream, &buffer[..count])?;
            let count = noise.read_message(&read_frame(&mut stream, 1024)?, &mut buffer)?;
            peer_offer = buffer[..count].to_vec();
        } else {
            let count = noise.read_message(&read_frame(&mut stream, 1024)?, &mut buffer)?;
            peer_offer = buffer[..count].to_vec();
            let count = noise.write_message(&offer, &mut buffer)?;
            write_frame(&mut stream, &buffer[..count])?;
        }
        Ok(Self {
            stream,
            noise: noise.into_transport_mode()?,
            deadline: Instant::now() + Duration::from_secs(10),
            compact: super::compact::supported(&peer_offer),
            peer_offer,
        })
    }
    fn encrypted_write(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(Instant::now() < self.deadline, "Sync deadline exceeded");
        let mut out = vec![0; bytes.len() + 16];
        let count = self.noise.write_message(bytes, &mut out)?;
        write_frame(&mut self.stream, &out[..count])
    }
    fn encrypted_read(&mut self) -> Result<Vec<u8>> {
        ensure!(Instant::now() < self.deadline, "Sync deadline exceeded");
        let bytes = read_frame(&mut self.stream, CHUNK + 16)?;
        let mut out = vec![0; bytes.len()];
        let count = self.noise.read_message(&bytes, &mut out)?;
        out.truncate(count);
        Ok(out)
    }
    pub fn send(&mut self, exchange: &Exchange) -> Result<()> {
        exchange.validate()?;
        let bytes = if self.compact {
            super::compact::encode(
                exchange,
                super::compact::offer(exchange)? == self.peer_offer,
            )?
        } else {
            serde_json::to_vec(exchange)?
        };
        ensure!(bytes.len() <= LIMIT, "Sync payload too large");
        self.encrypted_write(&(bytes.len() as u32).to_be_bytes())?;
        for chunk in bytes.chunks(CHUNK) {
            self.encrypted_write(chunk)?;
        }
        Ok(())
    }
    pub fn receive(&mut self) -> Result<Exchange> {
        let header = self.encrypted_read()?;
        ensure!(header.len() == 4, "Invalid sync length");
        let length = u32::from_be_bytes(header.try_into().unwrap()) as usize;
        ensure!(length <= LIMIT, "Sync payload too large");
        let mut bytes = Vec::with_capacity(length);
        while bytes.len() < length {
            let chunk = self.encrypted_read()?;
            ensure!(
                !chunk.is_empty() && chunk.len() <= length - bytes.len(),
                "Invalid sync chunk"
            );
            bytes.extend(chunk);
        }
        let exchange: Exchange = if self.compact {
            super::compact::decode(&bytes)?
        } else {
            serde_json::from_slice(&bytes)?
        };
        exchange.validate()?;
        Ok(exchange)
    }
}
fn read_frame(stream: &mut TcpStream, limit: usize) -> Result<Vec<u8>> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut size = [0; 4];
    read_before(stream, &mut size, deadline)?;
    let length = u32::from_be_bytes(size) as usize;
    ensure!(length > 0 && length <= limit, "Invalid sync frame size");
    let mut bytes = vec![0; length];
    read_before(stream, &mut bytes, deadline)?;
    Ok(bytes)
}
fn read_before(stream: &mut TcpStream, mut bytes: &mut [u8], deadline: Instant) -> Result<()> {
    while !bytes.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(!remaining.is_zero(), "Sync frame timed out");
        stream.set_read_timeout(Some(remaining))?;
        let count = stream.read(bytes)?;
        ensure!(count > 0, "Sync connection closed");
        bytes = &mut bytes[count..];
    }
    Ok(())
}
fn write_frame(stream: &mut TcpStream, bytes: &[u8]) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let size = (bytes.len() as u32).to_be_bytes();
    for mut remaining in [&size[..], bytes] {
        while !remaining.is_empty() {
            let timeout = deadline.saturating_duration_since(Instant::now());
            ensure!(!timeout.is_zero(), "Sync frame timed out");
            stream.set_write_timeout(Some(timeout))?;
            let count = stream.write(remaining)?;
            ensure!(count > 0, "Sync connection closed");
            remaining = &remaining[count..];
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};

    // The previous release sent no Noise payload and discarded the received one.
    fn legacy(
        mut stream: TcpStream,
        hello: &[u8],
        initiator: bool,
        offer: &[u8],
    ) -> Result<Channel> {
        configure(&stream)?;
        let builder = snow::Builder::new(PATTERN.parse()?)
            .psk(0, &[7; 32])?
            .prologue(hello)?;
        let mut noise = if initiator {
            builder.build_initiator()?
        } else {
            builder.build_responder()?
        };
        let mut buffer = [0; 1024];
        if initiator {
            let count = noise.write_message(offer, &mut buffer)?;
            write_frame(&mut stream, &buffer[..count])?;
            noise.read_message(&read_frame(&mut stream, 1024)?, &mut buffer)?;
        } else {
            noise.read_message(&read_frame(&mut stream, 1024)?, &mut buffer)?;
            let count = noise.write_message(offer, &mut buffer)?;
            write_frame(&mut stream, &buffer[..count])?;
        }
        Ok(Channel {
            stream,
            noise: noise.into_transport_mode()?,
            deadline: Instant::now() + Duration::from_secs(10),
            compact: false,
            peer_offer: vec![],
        })
    }

    #[test]
    fn compact_negotiation_and_legacy_fallback_work_in_both_directions() -> Result<()> {
        for (old_client, old_server, compact_v1) in [
            (false, false, false),
            (true, false, false),
            (false, true, false),
            (true, false, true),
            (false, true, true),
        ] {
            let mut offer = if compact_v1 {
                b"Carlitos compact 1\0".to_vec()
            } else {
                vec![]
            };
            if compact_v1 {
                offer.extend([0; 32]);
            }
            let server_offer = offer.clone();
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let address = listener.local_addr()?;
            let exchange = super::super::compact::tests::fixture();
            let server_exchange = Exchange {
                name: exchange.name.clone(),
                port: exchange.port,
                records: exchange.records.clone(),
                rates: exchange.rates.clone(),
            };
            let server = thread::spawn(move || -> Result<()> {
                let (mut stream, _) = listener.accept()?;
                let (_, hello) = read_hello(&mut stream)?;
                let mut channel = if old_server {
                    legacy(stream, &hello, false, &server_offer)?
                } else {
                    Channel::server(stream, &[7; 32], &hello, &server_exchange)?
                };
                let received = channel.receive()?;
                assert_eq!(channel.compact, !old_client && !old_server);
                assert_eq!(received.records.is_empty(), !old_client && !old_server);
                channel.send(&server_exchange)
            });
            let mut stream = TcpStream::connect(address)?;
            let hello = Hello {
                version: 1,
                from: "aa".repeat(16),
                to: "bb".repeat(16),
                invitation: String::new(),
            };
            let mut channel = if old_client {
                let hello = serde_json::to_vec(&hello)?;
                write_frame(&mut stream, &hello)?;
                legacy(stream, &hello, true, &offer)?
            } else {
                Channel::client(stream, &[7; 32], &hello, &exchange)?
            };
            assert_eq!(channel.compact, !old_client && !old_server);
            channel.send(&exchange)?;
            let received = channel.receive()?;
            if old_client || old_server {
                assert_eq!(received.records, exchange.records);
            } else {
                assert!(received.records.is_empty());
            }
            server.join().unwrap()?;
        }
        Ok(())
    }
}
