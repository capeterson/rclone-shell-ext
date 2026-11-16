// SPDX-License-Identifier: GPL-3.0-only
//! File-type icon *names* that match what file managers show: guess a MIME type from the file
//! *name*, then look up `type-subtype`, the generic icon for that type, and finally
//! `text-x-generic` in the current icon theme.
//!
//! Only names are inspected. Files live inside a FUSE mount, and reading their contents to
//! sniff a type would make rclone download them.

use mime_guess::mime::Mime;
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex, PoisonError},
};
use xdg_mime::SharedMimeInfo;

pub const FALLBACK_MIME_ICON: &str = "text-x-generic";
/// Bound for the per-name cache; the history never holds more than ~100 names at once.
const NAME_CACHE_LIMIT: usize = 512;

struct Cache {
    db: SharedMimeInfo,
    by_name: HashMap<String, Vec<String>>,
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| {
    Mutex::new(Cache {
        db: SharedMimeInfo::new(),
        by_name: HashMap::new(),
    })
});

/// The last path component.
pub fn basename(name: &str) -> &str {
    name.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(name)
}

pub fn guess(db: &SharedMimeInfo, name: &str) -> Mime {
    let base = basename(name);
    let guess = db.guess_mime_type().file_name(base).guess();
    // With only a name to go on, an unknown extension comes back as "zero-size file", which
    // would give every unrecognised file the empty-file icon.
    if guess.uncertain() || guess.mime_type().essence_str() == "application/x-zerosize" {
        mime_guess::from_path(base).first_or_octet_stream()
    } else {
        guess.mime_type().clone()
    }
}

/// Theme icon names to try, most specific first.
pub fn names_for(db: &SharedMimeInfo, mime: &Mime) -> Vec<String> {
    let mut names = db.lookup_icon_names(mime);
    if names.is_empty() {
        names.push(mime.essence_str().replace('/', "-"));
    }
    names.push(FALLBACK_MIME_ICON.to_string());
    names
}

/// Theme icon names to try for a remote path as rclone reports it (`Docs/report.pdf`), most
/// specific first. A trailing `/` means a directory.
pub fn icon_names(file_name: &str) -> Vec<String> {
    if file_name.ends_with('/') {
        return vec!["folder".to_string(), FALLBACK_MIME_ICON.to_string()];
    }
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(names) = cache.by_name.get(file_name) {
        return names.clone();
    }
    if cache.by_name.len() >= NAME_CACHE_LIMIT {
        cache.by_name.clear();
    }
    let mime = guess(&cache.db, file_name);
    let names = names_for(&cache.db, &mime);
    cache.by_name.insert(file_name.to_string(), names.clone());
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use mime_guess::mime;
    use std::path::Path;

    fn have_mime_db() -> bool {
        Path::new("/usr/share/mime/globs2").exists()
    }

    #[test]
    fn basename_takes_last_component() {
        assert_eq!(basename("a/b/c.txt"), "c.txt");
        assert_eq!(basename("c.txt"), "c.txt");
        assert_eq!(basename("dir/sub/"), "sub");
    }

    #[test]
    fn guess_ignores_directories_and_is_case_insensitive() {
        if !have_mime_db() {
            return;
        }
        let db = SharedMimeInfo::new();
        assert_eq!(
            guess(&db, "Docs/report.pdf").essence_str(),
            "application/pdf"
        );
        assert_eq!(guess(&db, "Photos/IMG_1.PNG").essence_str(), "image/png");
    }

    #[test]
    fn unknown_extension_is_octet_stream() {
        let db = SharedMimeInfo::new();
        assert_eq!(
            guess(&db, "mystery.zzzunknown").essence_str(),
            "application/octet-stream"
        );
    }

    #[test]
    fn names_end_with_the_universal_fallback() {
        let db = SharedMimeInfo::new();
        let names = names_for(&db, &mime::APPLICATION_PDF);
        assert_eq!(names.last().map(String::as_str), Some(FALLBACK_MIME_ICON));
        if have_mime_db() {
            assert!(names.iter().any(|n| n == "application-pdf"), "{names:?}");
        }
    }

    #[test]
    fn missing_database_still_yields_names() {
        // An empty database knows nothing; we must still produce something to try.
        let db = SharedMimeInfo::new_for_directory("/nonexistent-mime-dir");
        let names = names_for(&db, &mime::IMAGE_PNG);
        assert!(names.contains(&"image-png".to_string()), "{names:?}");
    }
}
