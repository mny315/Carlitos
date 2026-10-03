use std::collections::VecDeque;

/// Copy a decoder's packed little-endian samples without assuming alignment.
pub(crate) fn decode_pcm16(bytes: &[u8], channels: u16) -> anyhow::Result<Vec<i16>> {
    anyhow::ensure!(channels > 0, "Audio has no channels");
    anyhow::ensure!(
        bytes.len().is_multiple_of(usize::from(channels) * 2),
        "Partial PCM frame"
    );
    Ok(bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect())
}

/// Map audible (silence-compacted) PCM frames back to the source file.
#[derive(Default)]
pub(crate) struct Timeline {
    quiet: u64,
    kept: u64,
    skipped: u64,
    jumps: VecDeque<(u64, u64)>,
    reported_skip: u64,
    pub discontinuity: bool,
}
impl Timeline {
    pub fn filter(
        &mut self,
        pcm: &[i16],
        channels: usize,
        sample_rate: u32,
        enabled: bool,
    ) -> Vec<i16> {
        if !enabled {
            // Normal playback only copies complete frames; it does not need
            // the per-sample silence detector or per-frame vector extension.
            let frames = pcm.len() / channels;
            self.kept += frames as u64;
            if frames > 0 {
                self.quiet = 0;
            }
            return pcm[..frames * channels].to_vec();
        }
        let mut result = Vec::with_capacity(pcm.len());
        for frame in pcm.chunks_exact(channels) {
            let quiet = frame.iter().all(|s| s.unsigned_abs() < 59);
            if quiet && self.quiet >= u64::from(sample_rate) / 2 {
                self.skipped += 1;
                if let Some(last) = self.jumps.back_mut().filter(|j| j.0 == self.kept) {
                    last.1 = self.skipped;
                } else {
                    self.jumps.push_back((self.kept, self.skipped));
                }
            } else {
                result.extend_from_slice(frame);
                self.kept += 1;
            }
            self.quiet = if quiet {
                self.quiet.saturating_add(1)
            } else {
                0
            };
        }
        result
    }
    pub fn source_frame(&mut self, output: u64) -> u64 {
        while self.jumps.get(1).is_some_and(|j| j.0 <= output) {
            self.jumps.pop_front();
        }
        let skipped = self
            .jumps
            .front()
            .filter(|j| j.0 <= output)
            .map_or(0, |j| j.1);
        self.discontinuity |= skipped != self.reported_skip;
        self.reported_skip = skipped;
        output.saturating_add(skipped)
    }
}

#[cfg(test)]
mod tests;
