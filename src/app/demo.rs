use super::text;
use crate::library::*;

pub fn demo_library() -> Library {
    let mut library = Library::default();
    library.sources.push(Source {
        id: 100,
        uri: "file:///home/you/Audiobooks".into(),
    });
    for (i, (title, author, count)) in [
        ("Пикник на обочине", "Аркадий и Борис Стругацкие", 12),
        ("Невидимые города", "Итало Кальвино", 9),
        ("Левая рука тьмы", "Урсула Ле Гуин", 20),
        ("Маленький принц", "Антуан де Сент-Экзюпери", 27),
        ("Солярис", "Станислав Лем", 14),
        ("The Wind in the Willows", "Kenneth Grahame", 12),
    ]
    .iter()
    .enumerate()
    {
        let id = i as Id + 1;
        library.sources.push(Source {
            id,
            uri: format!("file:///home/you/Audiobooks/Book%20{id}"),
        });
        library.books.push(Book {
            id,
            source_id: id,
            title: (*title).into(),
            author: (*author).into(),
            cover: None,
        });
        for n in 0..*count {
            let part = id * 100 + n as Id;
            library.parts.push(Part {
                id: part,
                book_id: id,
                file_id: part,
                title: format!("{} {}", text("Глава", "Chapter"), n + 1),
                ordinal: n,
            });
            library.media.push(Media {
                id: part,
                source_id: id,
                duration: Some(1_680_000 + (n as u64 * 137_000)),
                ..Default::default()
            });
        }
        if i == 0 || i == 2 || i == 3 {
            library.progress.push(Progress {
                book_id: id,
                part_id: id * 100 + 3,
                position: 743_000,
                completed: i == 3,
                updated: now(),
            });
        }
    }
    library.session.current = Some(Target::Book(103));
    library.session.position = 743_000;
    library
}
