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
        if let Event::BooksChanged(selection) = &event
            && self
                .events
                .iter()
                .any(|old| matches!(old, Event::BooksChanged(newer) if newer > selection))
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
        | (Event::SyncStatus(_), Event::SyncStatus(_))
        | (Event::SyncIndexing(_), Event::SyncIndexing(_))
        | (Event::SyncReady(_), Event::SyncReady(_))
        | (Event::Notice(_), Event::Notice(_))
        | (Event::Scanning(..), Event::Scanning(..))
        | (Event::SourceUpdating(_), Event::SourceUpdating(_))
        | (Event::BooksChanged(_), Event::BooksChanged(_))
        | (Event::Quit, Event::Quit)
        | (Event::Hidden(_) | Event::Show, Event::Hidden(_) | Event::Show) => true,
        (Event::Imported(_, a), Event::Imported(_, b)) => a == b,
        (Event::BookEdited(a, _), Event::BookEdited(b, _))
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
        queue.push(Event::BookEdited(1, Default::default()));
        queue.push(Event::SourceUpdated(2, "Updated".into()));
        queue.push(Event::Imported(1, "owner".into()));
        queue.push(Event::Quit);
        for i in 0..1000 {
            queue.push(Event::Hidden(true));
            queue.push(Event::Scanning(false, String::new()));
            queue.push(Event::SourceUpdating(false));
            queue.push(Event::Notice(format!("Error {i}")));
        }
        let events = queue.take();
        assert!(matches!(&events[0], Event::CoverPrepared(9, Ok(path)) if path == "prepared.png"));
        assert!(matches!(events[1], Event::BookEdited(1, _)));
        assert!(matches!(events[2], Event::SourceUpdated(2, _)));
        assert!(matches!(events[3], Event::Imported(1, _)));
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
    fn unrelated_import_contexts_survive_late_file_results() {
        let mut queue = PendingEvents::default();
        queue.push(Event::Imported(3, "older picker".into()));
        queue.push(Event::Imported(4, "current picker".into()));
        queue.push(Event::Imported(4, "older picker".into()));
        let events = queue.take();
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], Event::Imported(4, owner) if owner == "current picker"));
        assert!(matches!(&events[1], Event::Imported(4, owner) if owner == "older picker"));
    }

    #[test]
    fn queued_batch_completions_keep_the_newest_selection_context() {
        let mut queue = PendingEvents::default();
        queue.push(Event::BooksChanged(5));
        queue.push(Event::Notice("keep notice".into()));
        queue.push(Event::BooksChanged(8));
        queue.push(Event::BooksChanged(7));
        let events = queue.take();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], Event::Notice(_)));
        assert!(matches!(events[1], Event::BooksChanged(8)));
    }

    #[test]
    fn latest_status_keeps_its_order_among_distinct_completed_operations() {
        let mut queue = PendingEvents::default();
        queue.push(Event::Hidden(true));
        queue.push(Event::SourceUpdated(1, "Old".into()));
        queue.push(Event::BookEdited(1, Default::default()));
        queue.push(Event::Show);
        queue.push(Event::SourceUpdated(2, "Other source".into()));
        queue.push(Event::BookEdited(2, Default::default()));
        queue.push(Event::SourceUpdated(1, "Latest".into()));
        queue.push(Event::Hidden(false));
        let events = queue.take();
        assert_eq!(events.len(), 5);
        assert!(matches!(events[0], Event::BookEdited(1, _)));
        assert!(matches!(events[1], Event::SourceUpdated(2, _)));
        assert!(matches!(events[2], Event::BookEdited(2, _)));
        assert!(matches!(&events[3], Event::SourceUpdated(1, text) if text == "Latest"));
        assert!(matches!(events[4], Event::Hidden(false)));
    }
}
