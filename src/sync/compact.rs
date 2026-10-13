//! Negotiated binary payload: no paths, JSON field names or hexadecimal expansion.
use super::{MAX_RECORDS, RateRecord, Record, protocol::Exchange};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};

const RECORD_SIZE: usize = 101;
const RATE_SIZE: usize = 64;
// Different offers fall back to JSON in both directions, including compact v1.
const OFFER: &[u8] = b"Carlitos compact 2\0";

fn rate_bytes(rate: &RateRecord, bytes: &mut Vec<u8>) -> Result<()> {
    rate.validate()?;
    bytes.extend(hex::decode(&rate.book)?);
    bytes.extend(rate.value.to_be_bytes());
    bytes.extend(rate.updated.to_be_bytes());
    bytes.extend(hex::decode(&rate.device)?);
    Ok(())
}

fn record_bytes(record: &Record, bytes: &mut Vec<u8>) -> Result<()> {
    record.validate()?;
    bytes.extend(hex::decode(&record.book)?);
    bytes.extend(hex::decode(&record.part)?);
    bytes.extend((record.occurrence as u32).to_be_bytes());
    bytes.extend(record.position.to_be_bytes());
    bytes.push(u8::from(record.completed) | (u8::from(record.reset) << 1));
    bytes.extend(record.updated.to_be_bytes());
    bytes.extend(hex::decode(&record.device)?);
    Ok(())
}

pub(super) fn offer(exchange: &Exchange) -> Result<Vec<u8>> {
    exchange.validate()?;
    // Local book IDs/order differ between libraries; compare canonical records.
    let mut records = exchange.records.iter().collect::<Vec<_>>();
    records.sort_by(|a, b| {
        a.book
            .cmp(&b.book)
            .then(a.part.cmp(&b.part))
            .then(a.occurrence.cmp(&b.occurrence))
            .then(a.revision().cmp(&b.revision()))
    });
    let mut bytes = Vec::with_capacity(records.len() * RECORD_SIZE);
    for record in records {
        record_bytes(record, &mut bytes)?;
    }
    let mut rates = exchange.rates.iter().collect::<Vec<_>>();
    rates.sort_by(|a, b| a.book.cmp(&b.book).then(a.revision().cmp(&b.revision())));
    bytes.extend((rates.len() as u16).to_be_bytes());
    for rate in rates {
        rate_bytes(rate, &mut bytes)?;
    }
    let mut offer = OFFER.to_vec();
    offer.extend(Sha256::digest(bytes));
    Ok(offer)
}

pub(super) fn supported(offer: &[u8]) -> bool {
    offer.len() == OFFER.len() + 32 && offer.starts_with(OFFER)
}

pub(super) fn encode(exchange: &Exchange, unchanged: bool) -> Result<Vec<u8>> {
    exchange.validate()?;
    let mut bytes = Vec::new();
    bytes.push(exchange.name.len() as u8);
    bytes.extend(exchange.name.as_bytes());
    bytes.extend(exchange.port.to_be_bytes());
    bytes.extend((if unchanged { 0 } else { exchange.records.len() } as u16).to_be_bytes());
    bytes.extend((if unchanged { 0 } else { exchange.rates.len() } as u16).to_be_bytes());
    if !unchanged {
        for record in &exchange.records {
            record_bytes(record, &mut bytes)?;
        }
        for rate in &exchange.rates {
            rate_bytes(rate, &mut bytes)?;
        }
    }
    Ok(bytes)
}

fn read<const N: usize>(reader: &mut Cursor<&[u8]>) -> Result<[u8; N]> {
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub(super) fn decode(bytes: &[u8]) -> Result<Exchange> {
    let mut reader = Cursor::new(bytes);
    let length = read::<1>(&mut reader)?[0] as usize;
    ensure!((1..=128).contains(&length), "Invalid sync name length");
    let mut name = vec![0; length];
    reader.read_exact(&mut name)?;
    let name = String::from_utf8(name)?;
    let port = u16::from_be_bytes(read(&mut reader)?);
    let count = u16::from_be_bytes(read(&mut reader)?) as usize;
    let rate_count = u16::from_be_bytes(read(&mut reader)?) as usize;
    ensure!(
        count <= MAX_RECORDS
            && rate_count <= MAX_RECORDS
            && bytes.len() == length + 7 + count * RECORD_SIZE + rate_count * RATE_SIZE,
        "Invalid compact sync length"
    );
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        let book = hex::encode(read::<32>(&mut reader)?);
        let part = hex::encode(read::<32>(&mut reader)?);
        let occurrence = u32::from_be_bytes(read(&mut reader)?) as usize;
        let position = u64::from_be_bytes(read(&mut reader)?);
        let flags = read::<1>(&mut reader)?[0];
        ensure!(flags <= 3, "Invalid sync flags");
        records.push(Record {
            book,
            part,
            occurrence,
            position,
            completed: flags & 1 != 0,
            reset: flags & 2 != 0,
            updated: i64::from_be_bytes(read(&mut reader)?),
            device: hex::encode(read::<16>(&mut reader)?),
        });
    }
    let mut rates = Vec::with_capacity(rate_count);
    for _ in 0..rate_count {
        rates.push(RateRecord {
            book: hex::encode(read::<32>(&mut reader)?),
            value: f64::from_be_bytes(read(&mut reader)?),
            updated: i64::from_be_bytes(read(&mut reader)?),
            device: hex::encode(read::<16>(&mut reader)?),
        });
    }
    let exchange = Exchange {
        name,
        port,
        records,
        rates,
    };
    exchange.validate()?;
    Ok(exchange)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(crate) fn fixture() -> Exchange {
        Exchange {
            name: "Телефон".into(),
            port: 12345,
            rates: vec![RateRecord {
                book: format!("{:064x}", 0),
                value: 1.75,
                updated: crate::library::now_millis(),
                device: "cc".repeat(16),
            }],
            records: (0..3)
                .map(|i| Record {
                    book: format!("{i:064x}"),
                    part: "bb".repeat(32),
                    occurrence: i,
                    position: if i == 2 { 0 } else { 4_567_890 },
                    completed: i == 1,
                    reset: i == 2,
                    updated: crate::library::now_millis(),
                    device: "cc".repeat(16),
                })
                .collect(),
        }
    }

    #[test]
    fn compact_roundtrip_preserves_rewind_reset_and_large_positions() -> Result<()> {
        let exchange = fixture();
        let encoded = encode(&exchange, false)?;
        let decoded = decode(&encoded)?;
        assert_eq!(decoded.name, exchange.name);
        assert_eq!(decoded.port, exchange.port);
        assert_eq!(decoded.records, exchange.records);
        assert_eq!(decoded.rates, exchange.rates);
        let json = serde_json::to_vec(&exchange)?;
        assert!(encoded.len() * 2 < json.len());
        let unchanged = encode(&exchange, true)?;
        assert!(decode(&unchanged)?.records.is_empty());
        assert!(decode(&unchanged)?.rates.is_empty());
        assert_eq!(unchanged.len(), exchange.name.len() + 7);
        println!(
            "Sync payload: JSON {} B; binary {} B; unchanged {} B",
            json.len(),
            encoded.len(),
            unchanged.len()
        );
        Ok(())
    }

    #[test]
    fn fingerprint_is_order_independent_and_detects_position_and_reset_changes() -> Result<()> {
        let mut exchange = fixture();
        let original = offer(&exchange)?;
        assert!(supported(&original));
        assert!(!supported(&[]));
        exchange.records.reverse();
        assert_eq!(original, offer(&exchange)?);
        exchange.records[1].position -= 1;
        assert_ne!(original, offer(&exchange)?);
        exchange.records[1].position += 1;
        exchange.records[0].reset = false;
        assert_ne!(original, offer(&exchange)?);
        exchange.records[0].reset = true;
        exchange.rates[0].value = 2.0;
        assert_ne!(original, offer(&exchange)?);
        Ok(())
    }

    #[test]
    fn truncated_oversized_and_invalid_binary_payloads_are_rejected() -> Result<()> {
        let exchange = fixture();
        let encoded = encode(&exchange, false)?;
        for length in 0..encoded.len() {
            assert!(decode(&encoded[..length]).is_err());
        }
        let mut invalid = encoded.clone();
        invalid.push(0);
        assert!(decode(&invalid).is_err());
        let mut invalid = encoded;
        invalid[exchange.name.len() + 7 + 76] = 255;
        assert!(decode(&invalid).is_err());
        Ok(())
    }
}
