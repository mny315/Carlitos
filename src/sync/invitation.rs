use super::valid_hex;
use anyhow::{Context, Result, ensure};
use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::{Ipv4Addr, SocketAddr};

const PREFIX: &str = "CARLITOS://PAIR/2/";

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Invitation {
    pub version: u32,
    pub device: String,
    pub ticket: String,
    pub key: String,
    pub addresses: Vec<SocketAddr>,
    #[serde(skip)]
    seed: Option<[u8; 16]>,
}

/// A random 128-bit secret: safe to enter without reducing the handshake to a PIN.
/// Only its public ticket is advertised on the LAN, never the code or key.
pub(super) struct PairingCode([u8; 16]);
impl PairingCode {
    pub fn parse(text: &str) -> Result<Self> {
        ensure!(text.len() <= 64, "Invalid device code");
        let normalized: String = text
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != '-')
            .collect();
        let bytes = BASE32_NOPAD
            .decode(normalized.to_ascii_uppercase().as_bytes())
            .context("Invalid device code")?;
        Ok(Self(
            bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid device code"))?,
        ))
    }
    fn derive(&self, purpose: &[u8]) -> String {
        let mut hash = Sha256::new();
        hash.update(purpose);
        hash.update(self.0);
        hex::encode(hash.finalize())
    }
    pub fn ticket(&self) -> String {
        self.derive(b"Carlitos pairing ticket v2\0")[..32].into()
    }
    pub fn invitation(&self, device: String, addresses: Vec<SocketAddr>) -> Invitation {
        Invitation {
            version: 1,
            device,
            ticket: self.ticket(),
            key: self.derive(b"Carlitos pairing key v2\0"),
            addresses,
            seed: Some(self.0),
        }
    }
}

impl Invitation {
    pub fn new(device: String, addresses: Vec<SocketAddr>) -> Result<Self> {
        let mut seed = [0; 16];
        getrandom::fill(&mut seed).map_err(|e| anyhow::anyhow!("Random generator: {e}"))?;
        Ok(PairingCode(seed).invitation(device, addresses))
    }
    pub fn parse(text: &str) -> Result<Self> {
        ensure!(text.len() <= 4096, "QR code is too long");
        let text = text.trim();
        let invitation: Self = if let Some(encoded) = text.to_ascii_uppercase().strip_prefix(PREFIX)
        {
            let bytes = BASE32_NOPAD
                .decode(encoded.as_bytes())
                .context("Invalid pairing QR code")?;
            ensure!(
                (32..=80).contains(&bytes.len()) && (bytes.len() - 32).is_multiple_of(6),
                "Invalid pairing QR code"
            );
            let seed = PairingCode(bytes[16..32].try_into().unwrap());
            let addresses = bytes[32..]
                .as_chunks::<6>()
                .0
                .iter()
                .map(|a| {
                    SocketAddr::from((
                        Ipv4Addr::new(a[0], a[1], a[2], a[3]),
                        u16::from_be_bytes([a[4], a[5]]),
                    ))
                })
                .collect();
            seed.invitation(hex::encode(&bytes[..16]), addresses)
        } else {
            // Existing installations' hex-encoded JSON invitations still work.
            let encoded = text
                .strip_prefix("carlitos://pair/")
                .context("This is not a Carlitos pairing QR code")?;
            serde_json::from_slice(&hex::decode(encoded)?)?
        };
        invitation.validate()?;
        Ok(invitation)
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1
                && valid_hex(&self.device, 16)
                && valid_hex(&self.ticket, 16)
                && valid_hex(&self.key, 32),
            "Invalid pairing QR code"
        );
        ensure!(
            self.addresses.len() <= 8 && self.addresses.iter().all(valid_address),
            "Invalid local device address"
        );
        Ok(())
    }
    pub fn code(&self) -> String {
        self.seed
            .map(|seed| {
                let code = BASE32_NOPAD.encode(&seed);
                format!(
                    "{}-{}-{}-{}-{}",
                    &code[..5],
                    &code[5..10],
                    &code[10..15],
                    &code[15..20],
                    &code[20..]
                )
            })
            .unwrap_or_default()
    }
    pub fn encode(&self) -> Result<String> {
        self.validate()?;
        if let Some(seed) = self.seed {
            let mut bytes = hex::decode(&self.device)?;
            bytes.extend_from_slice(&seed);
            for address in &self.addresses {
                if let SocketAddr::V4(address) = address {
                    bytes.extend_from_slice(&address.ip().octets());
                    bytes.extend_from_slice(&address.port().to_be_bytes());
                }
            }
            Ok(format!("{PREFIX}{}", BASE32_NOPAD.encode(&bytes)))
        } else {
            Ok(format!(
                "carlitos://pair/{}",
                hex::encode(serde_json::to_vec(self)?)
            ))
        }
    }
}

pub(super) fn valid_address(address: &SocketAddr) -> bool {
    match address {
        SocketAddr::V4(a) => {
            a.port() != 0
                && !a.ip().is_unspecified()
                && !a.ip().is_multicast()
                && !a.ip().is_broadcast()
        }
        SocketAddr::V6(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_qr_and_manual_code_recover_the_same_secret() -> Result<()> {
        let original = Invitation::new(
            "ab".repeat(16),
            vec!["192.168.1.190:39651".parse()?, "172.19.0.1:39651".parse()?],
        )?;
        let encoded = original.encode()?;
        assert!(encoded.len() < 100);
        assert!(encoded.bytes().all(|c| !c.is_ascii_lowercase()));
        let qr = Invitation::parse(&encoded)?;
        assert_eq!(qr.device, original.device);
        assert_eq!(qr.addresses, original.addresses);
        assert_eq!(qr.ticket, original.ticket);
        assert_eq!(qr.key, original.key);
        assert_eq!(original.code().replace('-', "").len(), 26);
        let manual = PairingCode::parse(&format!(
            " {} \n",
            original.code().to_lowercase().replace('-', " ")
        ))?;
        assert_eq!(manual.ticket(), qr.ticket);
        assert_eq!(manual.invitation(qr.device, qr.addresses).key, original.key);
        assert!(qrcode::QrCode::new(encoded)?.width() <= 37);
        Ok(())
    }

    #[test]
    fn legacy_qr_is_still_accepted() -> Result<()> {
        let value = serde_json::json!({"version":1,"device":"ab".repeat(16),
            "ticket":"cd".repeat(16),"key":"ef".repeat(32),"addresses":["192.168.1.10:39651"]});
        let encoded = format!(
            "carlitos://pair/{}",
            hex::encode(serde_json::to_vec(&value)?)
        );
        let parsed = Invitation::parse(&encoded)?;
        assert_eq!(parsed.key, "ef".repeat(32));
        assert_eq!(parsed.ticket, "cd".repeat(16));
        assert!(parsed.code().is_empty());
        assert_eq!(Invitation::parse(&parsed.encode()?)?.key, parsed.key);
        Ok(())
    }

    #[test]
    fn malformed_and_nonlocal_qr_payloads_are_rejected() -> Result<()> {
        for size in [0, 16, 31, 33, 79, 81, 86] {
            assert!(
                Invitation::parse(&format!("{PREFIX}{}", BASE32_NOPAD.encode(&vec![1; size])))
                    .is_err()
            );
        }
        for address in [
            "0.0.0.0:1",
            "224.0.0.1:1",
            "255.255.255.255:1",
            "192.168.1.1:0",
            "[::1]:10",
        ] {
            assert!(
                Invitation::new("ab".repeat(16), vec![address.parse()?])?
                    .encode()
                    .is_err()
            );
        }
        for code in ["123456", "", "OOOO-0000", &"A".repeat(4097)] {
            assert!(PairingCode::parse(code).is_err());
        }
        Ok(())
    }
}
