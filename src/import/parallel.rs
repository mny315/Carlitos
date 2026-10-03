//! Bounded document I/O, with stable output order and cooperative cancellation.
use super::ScanControl;
use anyhow::{Context, Result};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

const WORKERS: usize = 4;

pub(super) fn map<T: Sync, U: Send>(
    items: &[T],
    control: &ScanControl,
    read: impl Fn(&T) -> U + Sync,
) -> Result<Vec<U>> {
    control.check()?;
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        for _ in 0..WORKERS.min(items.len()) {
            let (tx, next, read) = (tx.clone(), &next, &read);
            std::thread::Builder::new()
                .name("carlitos-document".into())
                .spawn_scoped(scope, move || {
                    while control.check().is_ok() {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(index) else { break };
                        if tx.send((index, read(item))).is_err() {
                            break;
                        }
                    }
                })?;
        }
        drop(tx);
        let mut results: Vec<_> = (0..items.len()).map(|_| None).collect();
        for (index, result) in rx {
            results[index] = Some(result);
        }
        control.check()?;
        results
            .into_iter()
            .map(|result| result.context("Document worker did not finish"))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    #[test]
    fn document_work_is_bounded_parallel_and_preserves_input_order() -> Result<()> {
        let barrier = Barrier::new(WORKERS);
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let input: Vec<_> = (0..437).collect();
        let result = map(&input, &ScanControl::default(), |&index| {
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            // All four workers must actually run concurrently. A busy first
            // task must not stop later tasks from being picked up.
            if index < WORKERS {
                barrier.wait();
            }
            active.fetch_sub(1, Ordering::SeqCst);
            if index == 13 {
                Err("damaged recording")
            } else {
                Ok(index)
            }
        })?;
        assert_eq!(peak.load(Ordering::SeqCst), WORKERS);
        assert_eq!(result.len(), input.len());
        for (index, result) in result.into_iter().enumerate() {
            assert_eq!(
                result,
                if index == 13 {
                    Err("damaged recording")
                } else {
                    Ok(index)
                }
            );
        }
        Ok(())
    }

    #[test]
    fn cancelled_scan_stops_scheduling_documents_and_joins_inflight_work() {
        let control = ScanControl::default();
        let calls = AtomicUsize::new(0);
        let result = map(&[0; 437], &control, |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            control.cancel();
        });
        assert!(result.is_err());
        assert!((1..=WORKERS).contains(&calls.load(Ordering::SeqCst)));
        let result = map(&[0], &control, |_| panic!("cancelled work started"));
        assert!(result.is_err());
        assert!(
            map::<u8, ()>(&[], &ScanControl::default(), |_| ())
                .unwrap()
                .is_empty()
        );
    }
}
