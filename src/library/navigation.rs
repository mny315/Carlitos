use super::{Id, Library, Media, Millis, Part, Progress, Target, now};

impl Library {
    pub fn media(&self, id: Id) -> Option<&Media> {
        self.media.iter().find(|m| m.id == id)
    }
    pub fn part(&self, id: Id) -> Option<&Part> {
        self.parts.iter().find(|p| p.id == id)
    }
    pub fn book_parts(&self, id: Id) -> Vec<&Part> {
        let mut p: Vec<_> = self.parts.iter().filter(|p| p.book_id == id).collect();
        p.sort_by_key(|p| p.ordinal);
        p
    }
    pub fn file_for(&self, target: &Target) -> Option<&Media> {
        let id = match target {
            Target::Book(id) => self.part(*id)?.file_id,
        };
        self.media(id)
    }
    pub fn active_file(&self) -> Option<&Media> {
        self.file_for(self.session.current.as_ref()?)
    }
    /// Current part and the start of the chapter containing the playback position.
    /// A missing chapter start identifies the part row, including gaps in its TOC.
    pub fn active_chapter_key(&self, position: Millis) -> Option<(Id, Option<Millis>)> {
        let Target::Book(id) = self.session.current.as_ref()?;
        let file = self.active_file()?;
        let chapter = file
            .chapters
            .iter()
            .rev()
            .find(|c| c.start <= position && c.end.is_none_or(|end| position < end));
        Some((*id, chapter.map(|c| c.start)))
    }
    pub fn update_progress(&mut self, position: Millis, completed: bool) {
        self.session.position = position;
        if let Some(Target::Book(id)) = self.session.current
            && let Some(part) = self.part(id)
        {
            let progress = Progress {
                book_id: part.book_id,
                part_id: id,
                position,
                completed,
                updated: now(),
            };
            self.progress.retain(|p| p.book_id != progress.book_id);
            self.progress.push(progress);
        }
    }
    pub fn sequence(&self) -> Vec<Target> {
        match self.session.current {
            Some(Target::Book(id)) => self
                .part(id)
                .map(|p| {
                    self.book_parts(p.book_id)
                        .iter()
                        .map(|p| Target::Book(p.id))
                        .collect()
                })
                .unwrap_or_default(),
            None => vec![],
        }
    }
    pub fn neighbour(&self, forward: bool) -> Option<Target> {
        let seq = self.sequence();
        let index = seq
            .iter()
            .position(|t| Some(t) == self.session.current.as_ref())?;
        let next = if forward {
            index.checked_add(1)?
        } else {
            index.checked_sub(1)?
        };
        seq.get(next).cloned()
    }
}
