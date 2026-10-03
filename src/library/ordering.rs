use super::Media;
use std::cmp::Ordering;

pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    compare_digit_runs(&a.to_lowercase(), &b.to_lowercase()).then_with(|| a.cmp(b))
}

/// Compare already normalized strings without allocating in the sort callback.
/// Callers choose the final text tie-break for equal numeric runs.
pub(super) fn compare_digit_runs(a: &str, b: &str) -> Ordering {
    let (a_bytes, b_bytes) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a_bytes.len() && j < b_bytes.len() {
        if a_bytes[i].is_ascii_digit() && b_bytes[j].is_ascii_digit() {
            let (start_a, start_b) = (i, j);
            while i < a_bytes.len() && a_bytes[i].is_ascii_digit() {
                i += 1;
            }
            while j < b_bytes.len() && b_bytes[j].is_ascii_digit() {
                j += 1;
            }
            let na = a[start_a..i].trim_start_matches('0');
            let nb = b[start_b..j].trim_start_matches('0');
            let cmp = na.len().cmp(&nb.len()).then_with(|| na.cmp(nb));
            if cmp != Ordering::Equal {
                return cmp;
            }
        } else {
            let cmp = a_bytes[i].cmp(&b_bytes[j]);
            if cmp != Ordering::Equal {
                return cmp;
            }
            i += 1;
            j += 1;
        }
    }
    (a_bytes.len() - i).cmp(&(b_bytes.len() - j))
}
pub fn order_parts(parts: &mut [Media]) {
    let mut keys = std::collections::HashSet::new();
    let all_disc = parts.iter().all(|p| p.disc.is_some());
    let no_disc = parts.iter().all(|p| p.disc.is_none());
    let tagged = (all_disc || no_disc)
        && parts.iter().all(|p| {
            p.track.is_some_and(|t| t > 0)
                && keys.insert((p.disc.unwrap_or(1), p.track.unwrap_or(0)))
        });
    if tagged {
        parts.sort_by_key(|p| (p.disc.unwrap_or(1), p.track));
    } else {
        parts.sort_by(|a, b| natural_cmp(&a.relative, &b.relative));
    }
}
