// SPDX-License-Identifier: GPL-3.0-only
//! File-type icons for COSMIC: the icon names come from `rclone-ui-core`, this wraps them in
//! libcosmic icon handles.

use cosmic::widget::icon::{self, IconFallback};
use rclone_ui_core::mime_icon::{icon_names, FALLBACK_MIME_ICON};
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{LazyLock, Mutex, PoisonError},
};

type HandleCache = HashMap<(Vec<String>, u16), icon::Handle>;

static HANDLES: LazyLock<Mutex<HandleCache>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn handle(names: Vec<String>, size: u16) -> icon::Handle {
    let mut names = names.into_iter();
    let first = names
        .next()
        .unwrap_or_else(|| FALLBACK_MIME_ICON.to_string());
    let rest: Vec<Cow<'static, str>> = names.map(Cow::Owned).collect();
    icon::from_name(first)
        .prefer_svg(true)
        .size(size)
        .fallback(Some(IconFallback::Names(rest)))
        .handle()
}

/// Icon for a remote path as rclone reports it (`Docs/report.pdf`). A trailing `/` means a
/// directory.
pub fn file_icon(name: &str, size: u16) -> icon::Handle {
    let names = icon_names(name);
    let mut cache = HANDLES.lock().unwrap_or_else(PoisonError::into_inner);
    cache
        .entry((names.clone(), size))
        .or_insert_with(|| handle(names, size))
        .clone()
}
