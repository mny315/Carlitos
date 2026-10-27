//! Paired, local-network progress exchange. Audio and local paths never go on the wire.
mod compact;
mod fingerprint;
mod invitation;
mod network;
mod protocol;
#[cfg(test)]
mod tests;

use anyhow::{Result, ensure};
pub use fingerprint::{Fingerprint, Index};
pub use network::{Network, Request};
use serde::{Deserialize, Serialize};

pub const MAX_RECORDS: usize = 10_000;

/// Book speed has its own revision: changing it must not rewind playback.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RateRecord {
    pub book: String,
    pub value: f64,
    pub updated: i64,
    pub device: String,
}
impl RateRecord {
    pub fn revision(&self) -> (i64, &str) {
        (self.updated, &self.device)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            valid_hex(&self.book, 32) && valid_hex(&self.device, 16),
            "Invalid speed identity"
        );
        ensure!(
            self.value.is_finite()
                && (crate::player::MIN_RATE..=crate::player::MAX_RATE).contains(&self.value),
            "Invalid book speed"
        );
        ensure!(
            self.updated > 0
                && self.updated <= crate::library::now_millis().saturating_add(86_400_000),
            "Invalid speed date; check device clocks"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Record {
    pub book: String,
    pub part: String,
    pub occurrence: usize,
    pub position: u64,
    pub completed: bool,
    pub reset: bool,
    pub updated: i64,
    pub device: String,
}
impl Record {
    pub fn revision(&self) -> (i64, &str) {
        (self.updated, &self.device)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            valid_hex(&self.book, 32) && valid_hex(&self.part, 32),
            "Invalid book fingerprint"
        );
        ensure!(valid_hex(&self.device, 16), "Invalid device ID");
        ensure!(
            self.updated > 0
                && self.updated <= crate::library::now_millis().saturating_add(86_400_000),
            "Invalid progress date; check device clocks"
        );
        ensure!(
            self.position <= i64::MAX as u64 && self.occurrence < MAX_RECORDS,
            "Invalid progress position"
        );
        ensure!(
            !self.reset || (self.position == 0 && !self.completed),
            "Invalid progress reset"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub ready_books: Vec<crate::library::Id>,
    pub enabled: bool,
    pub running: bool,
    pub busy: bool,
    pub error: bool,
    pub name: String,
    pub invitation: String,
    pub code: String,
    pub peers: Vec<PeerStatus>,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerStatus {
    pub id: String,
    pub name: String,
    pub online: bool,
}

pub(crate) fn random_hex(bytes: usize) -> Result<String> {
    let mut value = vec![0; bytes];
    getrandom::fill(&mut value).map_err(|e| anyhow::anyhow!("Random generator: {e}"))?;
    Ok(hex::encode(value))
}
pub(crate) fn valid_hex(value: &str, bytes: usize) -> bool {
    value.len() == bytes * 2
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn qr_image(value: &str) -> Result<slint::Image> {
    let code = qrcode::QrCode::new(value.as_bytes())?;
    let width = code.width();
    let size = (width + 8) * 4;
    let mut pixels = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(size as u32, size as u32);
    for (i, pixel) in pixels.make_mut_slice().iter_mut().enumerate() {
        let x = i % size / 4;
        let y = i / size / 4;
        let dark = x >= 4
            && y >= 4
            && x < width + 4
            && y < width + 4
            && code[(x - 4, y - 4)] == qrcode::Color::Dark;
        let color = if dark { 0 } else { 255 };
        *pixel = slint::Rgb8Pixel::new(color, color, color);
    }
    Ok(slint::Image::from_rgb8(pixels))
}
