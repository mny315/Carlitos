use super::*;
use std::cmp::Ordering;
#[test]
fn source_counts_include_nested_books_once_and_match_path_boundaries() {
    let mut library = Library::default();
    for (id, path) in [
        (1, "/Audio"),
        (2, "/Audio/Book 1"),
        (3, "/Audio/Book 2"),
        (4, "/Audio2"),
        (5, "/Audio/Book 1/CD 1"),
    ] {
        library.sources.push(Source {
            id,
            uri: if cfg!(windows) {
                format!("file:///C:{path}").replace(' ', "%20")
            } else {
                format!("file://{path}").replace(' ', "%20")
            },
        });
    }
    for (id, source_id) in [(1, 2), (2, 3), (3, 4)] {
        library.books.push(Book {
            id,
            source_id,
            title: String::new(),
            author: String::new(),
            cover: None,
        });
    }
    for id in [1, 2] {
        library.media.push(Media {
            id,
            uri: format!(
                "file://{}/Audio/Book%201/CD%201/{id}.wav",
                if cfg!(windows) { "/C:" } else { "" }
            ),
            ..Default::default()
        });
        library.parts.push(Part {
            id,
            book_id: 1,
            file_id: id,
            title: String::new(),
            ordinal: id as usize,
        });
    }
    let counts = library.source_book_counts();
    assert_eq!(counts[&1], 2);
    for id in [2, 3, 4, 5] {
        assert_eq!(counts[&id], 1);
    }
}
#[test]
fn book_sorting_prioritizes_listening_and_places_missing_tags_last() {
    let mut library = Library::default();
    for (id, title, author, year, genre) in [
        (1, "Книга 10", "Борис", Some(2001), "Фантастика"),
        (2, "Книга 2", "Анна", None, ""),
        (3, "Альфа", "", Some(1990), "Детектив"),
        (4, "Завершена", "Вера", Some(2005), "Фантастика"),
    ] {
        library.books.push(Book {
            id,
            source_id: id,
            title: title.into(),
            author: author.into(),
            cover: None,
        });
        library.parts.push(Part {
            id,
            book_id: id,
            file_id: id,
            title: title.into(),
            ordinal: 0,
        });
        library.media.push(Media {
            id,
            year,
            genre: genre.into(),
            ..Default::default()
        });
    }
    for (id, completed) in [(1, false), (4, true)] {
        library.progress.push(Progress {
            book_id: id,
            part_id: id,
            position: 0,
            completed,
            updated: 1,
        });
    }
    let ids = |sort| {
        library
            .sorted_books(sort)
            .iter()
            .map(|b| b.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids("listening"), [1, 3, 2, 4]);
    assert_eq!(ids("title"), [3, 4, 2, 1]);
    assert_eq!(ids("author"), [2, 1, 4, 3]);
    assert_eq!(ids("year"), [3, 1, 4, 2]);
    assert_eq!(ids("genre"), [3, 4, 1, 2]);
    // Session targets are part IDs, not book IDs. Completed books and
    // books with no saved progress must also stay first when current.
    for part in &mut library.parts {
        part.id += 100;
    }
    for (sort, baseline) in [
        ("listening", vec![1, 3, 2, 4]),
        ("title", vec![3, 4, 2, 1]),
        ("author", vec![2, 1, 4, 3]),
        ("year", vec![3, 1, 4, 2]),
        ("genre", vec![3, 4, 1, 2]),
    ] {
        for current in [Some(101), Some(102), Some(104), Some(999), None] {
            library.session.current = current.map(Target::Book);
            let mut expected = baseline.clone();
            if let Some(index) = expected.iter().position(|id| Some(id + 100) == current) {
                let active = expected.remove(index);
                expected.insert(0, active);
            }
            assert_eq!(
                library
                    .sorted_books(sort)
                    .iter()
                    .map(|b| b.id)
                    .collect::<Vec<_>>(),
                expected,
                "{sort}, current part {current:?}",
            );
        }
    }
    // Tags can live on a later part, with unknown-duration recordings too.
    library.media[1].genre = "История".into();
    assert_eq!(library.book_tags()[&2].genre, "История");
}
#[test]
fn natural_order_unicode_and_long_numbers() {
    let mut a = [
        "Disc 2/10.mp3",
        "Disc 2/2.mp3",
        "Disc 1/10.mp3",
        "Disc 2/1.mp3",
    ];
    a.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(
        a,
        [
            "Disc 1/10.mp3",
            "Disc 2/1.mp3",
            "Disc 2/2.mp3",
            "Disc 2/10.mp3"
        ]
    );
    assert!(natural_cmp("глава 2", "глава 10").is_lt());
    assert!(natural_cmp("99999999999999999999", "100000000000000000000").is_lt());
    assert_ne!(natural_cmp("01", "1"), Ordering::Equal);
}
#[test]
fn incomplete_tags_use_whole_book_natural_order() {
    let mut parts = vec![
        Media {
            relative: "10.mp3".into(),
            track: Some(1),
            ..Default::default()
        },
        Media {
            relative: "2.mp3".into(),
            ..Default::default()
        },
    ];
    order_parts(&mut parts);
    assert_eq!(parts[0].relative, "2.mp3");
}
#[test]
fn book_boundaries_and_unknown_duration() {
    let d = [Some(10_000), Some(20_000), Some(5_000)];
    assert_eq!(book_seek(&d, 0, 9_000, 15_000), (1, 14_000, true));
    assert_eq!(book_seek(&d, 1, 2_000, -15_000), (0, 0, true));
    assert_eq!(book_seek(&d, 0, 0, 10_000), (1, 0, true));
    assert_eq!(book_seek(&d, 1, 0, 90_000), (2, 5_000, true));
    assert_eq!(
        book_seek(&[Some(10_000), None], 0, 9_000, 15_000),
        (0, 10_000, false)
    );
}
#[test]
fn exact_time_validation() {
    assert_eq!(parse_time("1:02:03", 4_000_000), Some(3_723_000));
    for invalid in [
        "1:60",
        "-1:00",
        "1",
        "2:00:00",
        "18446744073709551615:00",
        "1:+2",
    ] {
        assert_eq!(parse_time(invalid, 4_000_000), None);
    }
}
#[test]
fn chapter_highlight_follows_seek_boundaries_and_parts() {
    let mut l = Library::default();
    l.media.push(Media {
        id: 1,
        chapters: vec![
            Chapter {
                title: "First".into(),
                start: 1000,
                end: Some(5000),
            },
            Chapter {
                title: "Second".into(),
                start: 5000,
                end: Some(9000),
            },
        ],
        ..Default::default()
    });
    l.parts.push(Part {
        id: 2,
        book_id: 3,
        file_id: 1,
        title: String::new(),
        ordinal: 0,
    });
    assert_eq!(l.active_chapter_key(1000), None);
    l.session.current = Some(Target::Book(2));
    assert_eq!(l.active_chapter_key(0), Some((2, None)));
    assert_eq!(l.active_chapter_key(1000), Some((2, Some(1000))));
    assert_eq!(l.active_chapter_key(4999), Some((2, Some(1000))));
    assert_eq!(l.active_chapter_key(5000), Some((2, Some(5000))));
    assert_eq!(l.active_chapter_key(9000), Some((2, None)));
    l.media[0].chapters[0].start = 0;
    assert_eq!(l.active_chapter_key(0), Some((2, Some(0))));
    assert_eq!(l.active_chapter_key(9000), Some((2, None)));
    l.media[0].chapters.clear();
    assert_eq!(l.active_chapter_key(5000), Some((2, None)));
}
#[test]
fn book_navigation_stays_within_the_current_book() {
    let mut l = Library::default();
    for (id, book_id, ordinal) in [(1, 1, 1), (2, 2, 0), (3, 1, 0)] {
        l.parts.push(Part {
            id,
            book_id,
            file_id: id,
            title: String::new(),
            ordinal,
        });
    }
    assert_eq!(l.neighbour(true), None);
    l.session.current = Some(Target::Book(3));
    assert_eq!(l.neighbour(false), None);
    assert_eq!(l.neighbour(true), Some(Target::Book(1)));
    l.session.current = Some(Target::Book(1));
    assert_eq!(l.neighbour(true), None);
    assert_eq!(l.neighbour(false), Some(Target::Book(3)));
}
