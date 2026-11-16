// SPDX-License-Identifier: GPL-3.0-only
//! `Strings`: a QML singleton that hands the windows their static UI text (`Strings.t("id")`).
//! Messages with arguments or plurals never come through here; the Rust side formats those and
//! gives QML the finished string.

use cxx_qt_lib::QString;
use i18n_embed::fluent::FluentLanguageLoader;

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        type Strings = super::StringsRust;

        /// The text for a variable-free message id, in the user's language.
        #[qinvokable]
        fn t(self: &Strings, id: &QString) -> QString;
    }
}

#[derive(Default)]
pub struct StringsRust;

/// Look `id` up in `loader`. An id that does not exist comes back as the id itself: a visible,
/// greppable marker of a typo, instead of an empty label (or the loader's "No localization" text).
fn lookup(loader: &FluentLanguageLoader, id: &str) -> String {
    if loader.has(id) {
        loader.get(id)
    } else {
        id.to_owned()
    }
}

impl qobject::Strings {
    #[allow(clippy::unused_self)] // the QML invokable needs an object
    fn t(&self, id: &QString) -> QString {
        QString::from(lookup(rclone_ui_i18n::loader(), &id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_id_gives_the_english_text() {
        assert_eq!(
            lookup(rclone_ui_i18n::loader(), "common-continue"),
            "Continue"
        );
    }

    #[test]
    fn unknown_id_gives_the_id() {
        assert_eq!(lookup(rclone_ui_i18n::loader(), "no-such-id"), "no-such-id");
    }
}
