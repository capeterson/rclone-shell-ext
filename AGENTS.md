# Working in rclone-shell-ext

rclone status, offline folders and sharing for Linux desktop shells: a shared Rust integration
(`crates/`) with a COSMIC front end (`cosmic/`) and a KDE Plasma front end (`plasma/`).
Read `docs/architecture.md` first; it is the contract. The Plasma D-Bus contract is
`plasma/docs/dbus-interface.md`: change it there first.

## Where code goes

- Talks to rclone, systemd, the filesystem, or decides state → `crates/core`.
- Decides what a window or panel shows, or which buttons apply → `crates/view`, as plain Rust
  returning finished strings. If two front ends would write the same `match`, it goes here.
- CLI modes, the keeper and pin units → `crates/cli` (the `rclone-shell-ext` binary). Front-end
  binaries only add their own window/daemon modes.
- Widgets, QML, D-Bus serving, desktop integration files → `cosmic/` or `plasma/`.
- Nothing in `crates/` depends on libcosmic, iced, Qt or cxx-qt.
- Identity strings (app id, slug, unit names) come from `core::identity`; never spell them.

## UI text

Every user-visible string lives in `i18n/en/rclone_shell_ext.ftl`. No literals in widgets or QML.
- Rust: `rclone_ui_i18n::fl!("id", arg = value)`.
- Kirigami windows: `Strings.t("id")`. Plasmoid: `s["id"]` (from the daemon's `Strings()`).
  Dolphin C++: `DaemonClient::text("id")`.
- Anything with arguments or plurals is formatted in Rust and handed over finished.
- Persisted state (offline.json, config, history) and IPC payloads carry codes and enums, never
  translated text: translate at display time (`PinError::label()`, not a stored `fl!` result).
  The keeper runs under systemd, possibly in another locale than the UI.
- Never change case or concatenate translated fragments in code (`to_lowercase()`, `a + " " + b`);
  use one message with a Fluent selector or arguments so translators control the sentence.
- Share an id when the meaning and grammatical role are the same, not just the English text: a
  state ("Uploading"), a row label and a headline are separate ids even if they read the same.
- Ids are kebab-case with a surface prefix (`health-`, `panel-`, `details-`, `settings-`,
  `offline-`, `share-`, `drive-`, `cli-`, `common-`; `cosmic-`/`plasma-`/`dolphin-` only for
  shell-only controls). Reuse an existing message before adding one; change wording in one place.
- Not user-visible (logs, D-Bus enum values, debug output)? Literal is fine. A false positive in
  the sync test gets `// i18n-ignore: <reason>`, not a weaker test.
- `cargo test -p rclone-ui-i18n` runs the sync check.

## Commands

- `just ci`: everything CI runs, for the parts you have toolchains for. `just core::ci`,
  `just cosmic::ci`, `just plasma::ci` run one part.
- `cargo test` at the root only builds `crates/*`. Front ends: `cargo test -p <package>`.
- `just test-live`: against a throwaway `rclone rcd`.
- `just cosmic::install-user` / `just plasma::install-user`: installs into `~/.local`. After a
  Plasma install, restart Dolphin with `QT_PLUGIN_PATH` set.

## Rules

- Never run migration, install, `systemctl --user`, or offline-folder setup against the real
  `$HOME` from tests or scripts: inject roots and runners, use temp dirs.
- Never grab screenshots of the live desktop; render offscreen or ask.
- Clippy is pedantic with `-D warnings`; keep `cargo fmt` clean.
- Keep `Cargo.lock` committed; it pins libcosmic. Don't `cargo update` broadly.
- Comments explain why, in plain sentences; doc comments on every `pub` item in `crates/`.
