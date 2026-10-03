use std::{
    collections::HashMap,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
static ENGLISH: AtomicBool = AtomicBool::new(false);
static TRANSLATIONS: OnceLock<HashMap<String, String>> = OnceLock::new();
pub fn configure(language: &str) {
    #[cfg(target_os = "android")]
    let locale = crate::android::context()
        .configuration
        .lock()
        .unwrap()
        .language
        .clone();
    #[cfg(not(target_os = "android"))]
    let locale = std::env::var("LANGUAGE")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("LC_ALL").ok().filter(|s| !s.is_empty()))
        .or_else(|| std::env::var("LC_MESSAGES").ok().filter(|s| !s.is_empty()))
        .or_else(|| std::env::var("LANG").ok())
        .unwrap_or_else(system_locale);
    ENGLISH.store(
        match language {
            "en" => true,
            "ru" => false,
            _ => !locale.to_ascii_lowercase().starts_with("ru"),
        },
        Ordering::Relaxed,
    );
}
#[cfg(not(target_os = "android"))]
fn system_locale() -> String {
    #[cfg(windows)]
    {
        let mut name = [0u16; 85];
        let len = unsafe { windows::Win32::Globalization::GetUserDefaultLocaleName(&mut name) };
        if len > 1 {
            return String::from_utf16_lossy(&name[..len as usize - 1]);
        }
    }
    String::new()
}
pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}
pub fn book_count(count: usize) -> String {
    book_count_in(count, english())
}
fn book_count_in(count: usize, english: bool) -> String {
    let forms = if english {
        ["book", "books", "books"]
    } else {
        ["книга", "книги", "книг"]
    };
    format!("{count} {}", forms[plural_form(count, english)])
}
pub fn part_count(count: usize) -> String {
    part_count_in(count, english())
}
fn part_count_in(count: usize, english: bool) -> String {
    let forms = if english {
        ["part", "parts", "parts"]
    } else {
        ["часть", "части", "частей"]
    };
    format!("{count} {}", forms[plural_form(count, english)])
}
fn plural_form(count: usize, english: bool) -> usize {
    if english {
        usize::from(count != 1)
    } else if (11..=14).contains(&(count % 100)) {
        2
    } else {
        match count % 10 {
            1 => 0,
            2..=4 => 1,
            _ => 2,
        }
    }
}
pub fn tr(text: &'static str) -> &'static str {
    if !english() {
        return text;
    }
    TRANSLATIONS
        .get_or_init(|| {
            serde_json::from_str(include_str!("../data/locales/en.json"))
                .expect("checked translation catalog")
        })
        .get(text)
        .map(String::as_str)
        .unwrap_or(text)
}
include!(concat!(env!("OUT_DIR"), "/translations.rs"));

#[cfg(test)]
mod tests {
    #[test]
    fn counts_use_russian_and_english_plurals() {
        for (n, book, part) in [
            (0, "книг", "частей"),
            (1, "книга", "часть"),
            (2, "книги", "части"),
            (4, "книги", "части"),
            (5, "книг", "частей"),
            (11, "книг", "частей"),
            (12, "книг", "частей"),
            (14, "книг", "частей"),
            (21, "книга", "часть"),
            (22, "книги", "части"),
            (25, "книг", "частей"),
            (111, "книг", "частей"),
        ] {
            assert_eq!(super::book_count_in(n, false), format!("{n} {book}"));
            assert_eq!(super::part_count_in(n, false), format!("{n} {part}"));
            assert_eq!(
                super::book_count_in(n, true),
                format!("{n} {}", if n == 1 { "book" } else { "books" })
            );
            assert_eq!(
                super::part_count_in(n, true),
                format!("{n} {}", if n == 1 { "part" } else { "parts" })
            );
        }
    }
    #[test]
    fn catalog_preserves_format_parameters() {
        let translations: std::collections::HashMap<String, String> =
            serde_json::from_str(include_str!("../data/locales/en.json")).unwrap();
        fn parameters(s: &str) -> Vec<&str> {
            s.split('{')
                .skip(1)
                .filter_map(|s| s.split_once('}').map(|(p, _)| p))
                .collect()
        }
        for (ru, en) in translations {
            assert_eq!(parameters(&ru), parameters(&en), "{ru}");
            assert!(!en.is_empty());
        }
    }
}
