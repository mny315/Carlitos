use std::sync::atomic::{AtomicU64, Ordering};

/// One coalesced Back per UI lifetime. The low bit marks a pending request;
/// lifecycle changes advance the generation and clear that bit atomically.
pub(super) struct BackRequests(AtomicU64);

impl BackRequests {
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    pub fn invalidate(&self) {
        let _ = self
            .0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |state| {
                Some((state & !1).wrapping_add(2))
            });
    }

    pub fn claim(&self) -> Option<u64> {
        let state = self.0.load(Ordering::SeqCst);
        if state & 1 != 0 {
            return None;
        }
        // Do not retry across a lifecycle change: this input belongs to the
        // generation observed at entry, rather than the newly attached UI.
        self.0
            .compare_exchange(state, state | 1, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| state | 1)
    }

    pub fn is_current(&self, request: u64) -> bool {
        self.0.load(Ordering::SeqCst) == request
    }

    pub fn release(&self, request: u64) {
        let _ = self
            .0
            .compare_exchange(request, request & !1, Ordering::SeqCst, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_input_routes_share_one_pending_back() {
        let requests = BackRequests::new();
        let first = requests.claim().unwrap();
        assert!(requests.claim().is_none());
        assert!(requests.is_current(first));
        requests.release(first);
        assert!(requests.claim().is_some());
    }

    #[test]
    fn old_callbacks_cannot_navigate_or_release_a_recreated_uis_back() {
        let requests = BackRequests::new();
        let old = requests.claim().unwrap();
        requests.invalidate(); // Detach.
        requests.invalidate(); // Attach a new Activity.
        let current = requests.claim().unwrap();
        assert!(!requests.is_current(old));
        requests.release(old);
        assert!(requests.is_current(current));
        assert!(requests.claim().is_none());
        requests.release(current);
        assert!(requests.claim().is_some());
    }
}
