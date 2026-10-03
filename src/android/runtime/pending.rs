use crate::app::Event;
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct PendingEvents {
    events: VecDeque<Event>,
}

impl PendingEvents {
    pub fn push(&mut self, event: Event) {
        // Cover workers can finish out of order. Request IDs increase across
        // Activities, so only the newest queued request can still own an editor.
        if let Event::CoverPrepared(request, _) = &event
            && self
                .events
                .iter()
                .any(|old| matches!(old, Event::CoverPrepared(newer, _) if newer > request))
        {
            return;
        }
        // Bound repeated background statuses by meaning, not arrival count.
        // Never evict an unrelated picker/worker result or completion: there
        // may be no Activity to consume it until much later. Retained events
        // keep their relative order; replacements occupy their new position.
        self.events.retain(|old| !supersedes(&event, old));
        self.events.push_back(event);
    }

    pub fn take(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }
}

fn supersedes(new: &Event, old: &Event) -> bool {
    match (new, old) {
        (Event::CoverPrepared(..), Event::CoverPrepared(..))
        | (Event::Notice(_), Event::Notice(_))
        | (Event::Scanning(..), Event::Scanning(..))
        | (Event::SourceUpdating(_), Event::SourceUpdating(_))
        | (Event::Imported, Event::Imported)
        | (Event::Quit, Event::Quit)
        | (Event::Hidden(_) | Event::Show, Event::Hidden(_) | Event::Show) => true,
        (Event::BookEdited(a), Event::BookEdited(b))
        | (Event::SourceUpdated(a, _), Event::SourceUpdated(b, _)) => a == b,
        #[cfg(target_os = "android")]
        (Event::Picked(a, ..), Event::Picked(b, ..)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_results_survive_a_burst_of_background_statuses() {
        let mut queue = PendingEvents::default();
        queue.push(Event::CoverPrepared(9, Ok("prepared.png".into())));
        queue.push(Event::BookEdited(1));
        queue.push(Event::SourceUpdated(2, "Updated".into()));
        queue.push(Event::Imported);
        queue.push(Event::Quit);
        for i in 0..1000 {
            queue.push(Event::Hidden(true));
            queue.push(Event::Scanning(false, String::new()));
            queue.push(Event::SourceUpdating(false));
            queue.push(Event::Notice(format!("Error {i}")));
        }
        let events = queue.take();
        assert!(matches!(&events[0], Event::CoverPrepared(9, Ok(path)) if path == "prepared.png"));
        assert!(matches!(events[1], Event::BookEdited(1)));
        assert!(matches!(events[2], Event::SourceUpdated(2, _)));
        assert!(matches!(events[3], Event::Imported));
        assert!(matches!(events[4], Event::Quit));
        assert_eq!(events.len(), 9);
        assert!(matches!(&events[8], Event::Notice(text) if text == "Error 999"));
        assert!(queue.take().is_empty());
    }

    #[test]
    fn later_cover_requests_win_even_when_older_workers_finish_last() {
        let mut queue = PendingEvents::default();
        queue.push(Event::CoverPrepared(5, Ok("old.png".into())));
        queue.push(Event::Notice("keep this notice".into()));
        queue.push(Event::CoverPrepared(8, Ok("new.png".into())));
        queue.push(Event::CoverPrepared(7, Err(anyhow::anyhow!("stale error"))));
        let events = queue.take();
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], Event::Notice(_)));
        assert!(matches!(&events[1], Event::CoverPrepared(8, Ok(path)) if path == "new.png"));
    }

    #[test]
    fn latest_status_keeps_its_order_among_distinct_completed_operations() {
        let mut queue = PendingEvents::default();
        queue.push(Event::Hidden(true));
        queue.push(Event::SourceUpdated(1, "Old".into()));
        queue.push(Event::BookEdited(1));
        queue.push(Event::Show);
        queue.push(Event::SourceUpdated(2, "Other source".into()));
        queue.push(Event::BookEdited(2));
        queue.push(Event::SourceUpdated(1, "Latest".into()));
        queue.push(Event::Hidden(false));
        let events = queue.take();
        assert_eq!(events.len(), 5);
        assert!(matches!(events[0], Event::BookEdited(1)));
        assert!(matches!(events[1], Event::SourceUpdated(2, _)));
        assert!(matches!(events[2], Event::BookEdited(2)));
        assert!(matches!(&events[3], Event::SourceUpdated(1, text) if text == "Latest"));
        assert!(matches!(events[4], Event::Hidden(false)));
    }
}
