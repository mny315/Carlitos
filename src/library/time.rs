use super::Millis;

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn format_time(ms: Millis) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}
pub fn parse_time(text: &str, duration: Millis) -> Option<Millis> {
    let fields: Vec<_> = text.trim().split(':').collect();
    if !(2..=3).contains(&fields.len()) {
        return None;
    }
    let mut secs = 0u64;
    for (i, part) in fields.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let n: u64 = part.parse().ok()?;
        if i > 0 && n >= 60 {
            return None;
        }
        secs = secs.checked_mul(60)?.checked_add(n)?;
    }
    let ms = secs.checked_mul(1000)?;
    (ms <= duration).then_some(ms)
}
/// Signed arithmetic is widened to avoid overflow; an exact internal boundary belongs to the next part.
pub fn book_seek(
    durations: &[Option<Millis>],
    index: usize,
    position: Millis,
    delta: i64,
) -> (usize, Millis, bool) {
    if index >= durations.len() {
        return (index, position, false);
    }
    if durations.iter().any(Option::is_none) {
        return (
            index,
            (position as i128 + delta as i128)
                .clamp(0, durations[index].unwrap_or(u64::MAX) as i128) as u64,
            false,
        );
    }
    let total: i128 = durations.iter().map(|d| d.unwrap_or(0) as i128).sum();
    let before: i128 = durations[..index]
        .iter()
        .map(|d| d.unwrap_or(0) as i128)
        .sum();
    let mut target = (before + position as i128 + delta as i128).clamp(0, total);
    for (i, duration) in durations.iter().enumerate() {
        let d = duration.unwrap_or(0) as i128;
        if target < d || i == durations.len() - 1 {
            return (i, target.min(d) as u64, true);
        }
        target -= d;
    }
    (index, position, true)
}
