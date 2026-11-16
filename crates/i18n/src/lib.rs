// SPDX-License-Identifier: GPL-3.0-only
//! The UI text of the rclone shell extension: one Fluent file (`i18n/en/rclone_shell_ext.ftl` at
//! the repository root) used by every surface.
//!
//! # Using `fl!` from another crate
//!
//! [`fl!`] wraps `i18n_embed_fl::fl!` with the shared [`loader`], so message ids and argument
//! names are checked at compile time. `i18n_embed_fl` finds the Fluent files by reading the
//! `i18n.toml` of the crate that *invokes* the macro (via `CARGO_MANIFEST_DIR`), so every crate
//! that calls `fl!` needs this next to its `Cargo.toml`:
//!
//! ```toml
//! fallback_language = "en"
//!
//! [fluent]
//! assets_dir = "../../i18n"   # relative to that crate; the repo-root `i18n/` directory
//! domain = "rclone_shell_ext"
//! ```
//!
//! The `domain` override makes the macro read `i18n/en/rclone_shell_ext.ftl` instead of a file
//! named after the calling package. Call [`init`] once at start-up to pick the user's languages.

use fluent_syntax::ast;
use i18n_embed::fluent::{fluent_language_loader, FluentLanguageLoader};
use i18n_embed::{DesktopLanguageRequester, LanguageLoader};
use rust_embed::RustEmbed;
use std::collections::BTreeMap;
use std::sync::{LazyLock, Once};

// Re-exported so the `fl!` expansion works in crates that don't depend on it themselves.
#[doc(hidden)]
pub use i18n_embed_fl;

#[derive(RustEmbed)]
#[folder = "../../i18n"]
struct Localizations;

static LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader = fluent_language_loader!();
    // The fallback language is always available, so a missing locale never leaves us blank.
    loader
        .load_languages(&Localizations, &[loader.fallback_language().clone()])
        .expect("the embedded fallback language must load");
    // Isolation marks around arguments would show up in plain-text UIs and in comparisons.
    loader.set_use_isolating(false);
    loader
});

/// The shared loader. Messages resolve against the fallback language until [`init`] selects
/// others.
#[doc(hidden)]
pub fn loader() -> &'static FluentLanguageLoader {
    &LOADER
}

/// Select the user's desktop languages (falling back to English per message). Safe to call more
/// than once; only the first call does anything.
pub fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let requested = DesktopLanguageRequester::requested_languages();
        if let Err(e) = i18n_embed::select(&*LOADER, &Localizations, &requested) {
            eprintln!("rclone-ui-i18n: could not load the requested languages: {e}");
            // i18n-ignore: log
        }
        // Loading new bundles resets the setting.
        LOADER.set_use_isolating(false);
    });
}

/// Look up a message, with compile-time checking of the id and its arguments:
/// `fl!("common-continue")`, `fl!("offline-folders-count", count = n)`.
///
/// The calling crate needs an `i18n.toml`; see the crate documentation.
#[macro_export]
macro_rules! fl {
    ($id:literal $(, $($args:tt)*)?) => {
        $crate::i18n_embed_fl::fl!($crate::loader(), $id $(, $($args)*)?)
    };
}

/// Every message id in the fallback language, sorted. The sync test uses this to check that the
/// ids the code refers to exist and that none is left unused.
pub fn message_ids() -> Vec<String> {
    let loader = loader();
    let mut ids: Vec<String> = loader.with_message_iter(loader.fallback_language(), |messages| {
        messages.map(|m| m.id.name.to_string()).collect()
    });
    ids.sort();
    ids
}

/// Every message that has no variables, resolved for the current language, keyed by id. This is
/// what the Plasma daemon exports over D-Bus for the plasmoid and the Dolphin plugins.
pub fn static_messages() -> BTreeMap<String, String> {
    let loader = loader();
    let ids: Vec<String> = loader.with_message_iter(loader.fallback_language(), |messages| {
        messages
            .filter(|m| !message_has_variables(m))
            .map(|m| m.id.name.to_string())
            .collect()
    });
    ids.into_iter()
        .map(|id| {
            let text = loader.get(&id);
            (id, text)
        })
        .collect()
}

/// Whether the message (value or any attribute) refers to a variable, directly or inside a
/// selector or nested placeable. References to other messages or terms are not followed.
fn message_has_variables(message: &ast::Message<&str>) -> bool {
    let in_pattern = |p: &ast::Pattern<&str>| pattern_has_variables(p);
    message.value.as_ref().is_some_and(in_pattern)
        || message.attributes.iter().any(|a| in_pattern(&a.value))
}

fn pattern_has_variables(pattern: &ast::Pattern<&str>) -> bool {
    pattern.elements.iter().any(|e| match e {
        ast::PatternElement::TextElement { .. } => false,
        ast::PatternElement::Placeable { expression } => expression_has_variables(expression),
    })
}

fn expression_has_variables(expression: &ast::Expression<&str>) -> bool {
    match expression {
        ast::Expression::Inline(inline) => inline_has_variables(inline),
        ast::Expression::Select { selector, variants } => {
            inline_has_variables(selector)
                || variants.iter().any(|v| pattern_has_variables(&v.value))
        }
    }
}

fn inline_has_variables(inline: &ast::InlineExpression<&str>) -> bool {
    match inline {
        ast::InlineExpression::VariableReference { .. } => true,
        ast::InlineExpression::Placeable { expression } => expression_has_variables(expression),
        ast::InlineExpression::FunctionReference { arguments, .. }
        | ast::InlineExpression::TermReference {
            arguments: Some(arguments),
            ..
        } => arguments
            .positional
            .iter()
            .chain(arguments.named.iter().map(|n| &n.value))
            .any(inline_has_variables),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_only_variable_free_messages() {
        let all = static_messages();
        assert!(all.contains_key("common-continue"));
        assert!(!all.contains_key("offline-folders-count"));
    }

    #[test]
    fn init_is_idempotent() {
        init();
        init();
        assert_eq!(loader().get("common-continue"), "Continue");
    }
}
