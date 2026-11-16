# rclone-shell-ext architecture

One repository, one shared rclone integration, and thin desktop-shell front ends on top of it.

```
crates/core   rclone-ui-core    talking to rclone: rc client, status, config model + store,
                                offline folders (keeper, pins, units), share, drive, migration
crates/i18n   rclone-ui-i18n    the UI text: Fluent files, loader, fl! macro, export for QML/C++
crates/view   rclone-ui-view    presentation model: what to show and which actions apply,
                                as plain Rust (strings come from i18n, never literals)
crates/cli    rclone-shell-ext  the shared binary: keeper, pin units, offline/status/share CLI
cosmic/       rclone-shell-ext-cosmic        libcosmic applet + windows (widgets only)
plasma/daemon rclone-shell-ext-plasma        zbus daemon (+ window launcher)
plasma/ui     rclone-shell-ext-plasma-ui     cxx-qt bridge + Kirigami QML windows
plasma/plasmoid, plasma/dolphin              QML plasmoid, C++ Dolphin plugins
resources/                                   the only copy of every SVG
i18n/en/rclone_shell_ext.ftl                 the only copy of every user-visible string
```

Dependency direction is strictly downward: `cosmic`, `plasma/*` → `view` → `core` → `i18n`.
`cli` depends on `core` (and `i18n`). Nothing under `crates/` may depend on a toolkit
(libcosmic, iced, Qt, cxx-qt, zbus-as-a-server). A front end never re-implements logic that
`core` or `view` already has: if two front ends would write the same `match`, it belongs in `view`.

## Identity

Both front ends share **one identity** and therefore one set of state on a machine:

| Thing | Name |
|---|---|
| App id (`identity().app_id`) | `com.ca-peterson.rclone-shell-ext` |
| Slug (`identity().slug`) | `rclone-shell-ext` |
| Dev build | slug `rclone-shell-ext-dev`, app id `com.ca-peterson.rclone-shell-ext-dev` |
| Config | `$XDG_CONFIG_HOME/rclone-shell-ext/config.toml` (`core::config::TomlStore`) |
| Data / state | `$XDG_DATA_HOME/rclone-shell-ext/`, `$XDG_STATE_HOME/rclone-shell-ext/` |
| systemd units | `rclone-shell-ext-offline.service`, `rclone-shell-ext-pin@.service`, drop-in `50-rclone-shell-ext.conf`; all `ExecStart` the shared `rclone-shell-ext` binary |
| COSMIC desktop id | `com.ca-peterson.rclone-shell-ext.Cosmic` |
| Plasma desktop id (settings launcher) | `com.ca-peterson.rclone-shell-ext.Plasma` |
| Plasmoid id | `com.ca-peterson.rclone-shell-ext` |
| D-Bus (Plasma daemon) | name `com.ca-peterson.rclone-shell-ext`, path `/com/ca_peterson/rclone_shell_ext`, interface `com.ca_peterson.RcloneShellExt1` (interface names cannot contain `-`) |
| Icons | app `com.ca-peterson.rclone-shell-ext[-symbolic]`, status `rclone-shell-ext-<state>`, emblems `rclone-shell-ext-emblem-<state>` |

`core::identity` has **no default**: calling `identity()` before `set_identity()` panics with a
clear message. Tests: `core`'s own unit tests get the release identity automatically; other
crates call `identity::use_test_identity()` or enable core's `test-identity` feature from
`[dev-dependencies]`. The identity constants live in
`core::identity::{APP_ID, SLUG, DEV_APP_ID, DEV_SLUG}` (plus the `*_DESKTOP_ID`s) so no front end
spells them itself. A dev build is made with `APP_ID_OVERRIDE=com.ca-peterson.rclone-shell-ext-dev`
(`Identity::from_override`); its COSMIC desktop id is `com.ca-peterson.rclone-shell-ext-dev.Cosmic`
and its units run the `rclone-shell-ext-dev` binary: the units always `ExecStart` the sibling of
the running executable named after the slug (else `PATH`), see `core::units::shared_exe`.

Config is one TOML file both shells read and write. `TomlStore` takes an exclusive `flock` on a
sidecar `config.toml.lock` around every save (so the two shells and the CLI can't lose each
other's edits), writes atomically (temp file, fsync, rename), and **refuses to overwrite a file
it can't parse**: `load_checked()` reports the corrupt file (the Plasma `GetConfig` turns that
into a finished `errorText`), `load()` falls back to defaults for display only, and `save()`
fails instead of replacing the user's file with defaults. Front ends that need live updates
watch the file (mtime poll is fine; the Plasma daemon already polls every 2 s). COSMIC-only or
Plasma-only settings are still fields of the one `Config` struct.

### Migration from the old identities

`core::migrate` moves an old install over once. The marker is
`$XDG_STATE_HOME/rclone-shell-ext/migrated-v1`. Old identities:

| Old | slug | config |
|---|---|---|
| COSMIC applet | `cosmic-rclone-ui` | cosmic-config `~/.config/cosmic/com.ca-peterson.CosmicRcloneUi/v1/<field>` (one RON value per file) |
| Plasma | `rclone-plasma` | KConfig INI `~/.config/rclone-plasmarc`, group `[General]` |

Rules: pick the old install that has `offline_enabled` / units installed (if both have offline
folders and both keepers are enabled it is ambiguous and the migration aborts); never delete user
data; stop the old keeper and pin units before moving anything; **wait until quiet** (rclone has
no unfinished rc jobs and no FUSE mount sits on the old data or inside the Drive mount, up to a
timeout, then fail with `sync-in-flight` / `mount-in-use` and put the old keeper back); move
`data/` and `state/` dirs with `rename` (same filesystem) and **rename the bisync listing files
whose names embed the old data path** (through `bisync_name::flatten`, the same function the
keeper uses to find them) so the sync baseline survives (no `--resync`); rewrite units + drop-in
under the new names, `daemon-reload`, re-enable the new keeper (and start it if the Drive mount
runs); best-effort rewrite of the COSMIC panel applet entry. Any unexpected state: abort before
the first mutation, leave the old install untouched, and report. The old config files
(`rclone-plasmarc`, the cosmic-config directory) are left in place.

**Journal.** Before the first step the plan is written to
`$XDG_STATE_HOME/rclone-shell-ext-migrate-journal.json`; steps are idempotent, so an interrupted
run resumes from the journal when run again (and the journal, not a fresh scan, then decides which
old install is being moved). A failed attempt leaves a `<slug>-migrate-error` code file next to it
for `pending()`.

**When it runs.**
- *At install, in the foreground:* `just cosmic::install-user` / `just plasma::install-user`
  run `rclone-shell-ext --migrate`; a failure stops the recipe with the reason and leaves the old
  files (and the old plasmoid/daemon) working. Only after success are the old install's own files
  removed.
- *Plasma panel entry, at install, with plasmashell stopped:* `--migrate` does not touch
  `plasma-org.kde.plasma.desktop-appletsrc`, because plasmashell holds the layout in memory and
  writes it back over any edit made while it runs. `plasma::install-user` installs the new
  plasmoid, stops plasmashell, runs `rclone-shell-ext --plasma-panel-rename`
  (`migrate::plasma_panel::rewrite`, which refuses while a `plasmashell` process of the user runs),
  starts plasmashell again, and only then removes the old plasmoid package. The rewrite renames
  `plugin=` and the old id in comma lists (de-duplicated); when a system tray already has a
  new-id entry (plasmashell adds one when it finds the new plugin), the old tray entry is dropped
  with its sub-groups instead. It handles both the nested tray and a separate systemtray
  containment, and is idempotent.
- *CLI modes:* `rclone-shell-ext-cli`'s `startup(args)` (called by every binary) runs
  `migrate::run_if_needed()` before the command-line modes that may use offline folders, except
  in the unit modes (the migration stops those very units) and for `--migrate` itself.
- *Windowed front ends, in the background:* the Plasma daemon (after it owns its bus name, so
  D-Bus activation never waits) and the COSMIC applet call `migrate::spawn_background()`; both
  return immediately.
- A lock file keeps two processes starting together from both migrating (the second waits up to
  a minute, then reports `lock-timeout`). `RCLONE_SHELL_EXT_NO_MIGRATE` switches migration off for
  tests that run binaries; the dev build never migrates; a test run (a test identity was used,
  or core's own `cfg(test)`) behaves as if nothing was pending.

**`pending()` and `guard()`.** `migrate::pending()` is a few `stat`s and changes nothing: it
returns `NotNeeded`, `Pending` or `Failed(reason)`, and `.label()` is the finished text. The COSMIC
popup shows a banner row with the label while it is not `NotNeeded` (refreshed with each status
poll); the Plasma daemon puts `migration` (code) and `migrationLabel` (text) in `StatusJson` and the
plasmoid shows a banner. `migrate::guard()` refuses with `OldInstallPresent` while something is
pending; `keeper::set_pinned` (when pinning), `offline_setup::set_up` and `turn_on` all call it, so
the new keeper and the old one never fight over the same folders. The migration itself never calls
any of them. Everything in the migration takes injected roots (XDG dirs, a `systemctl` runner, rc
and mount probes) and is tested against temp dirs; it is never exercised against `$HOME` in tests
or by agents.

## UI text (string externalization)

Goal: one wording, defined once, used by every surface (iced widgets, Kirigami QML, the
plasmoid, the Dolphin C++ plugins, CLI output), with CI catching drift.

**Source of truth**: `i18n/<lang>/rclone_shell_ext.ftl` (Project Fluent). Only `en` exists
today; the layout is ready for more. Message ids are kebab-case and prefixed by surface:

| Prefix | Used for |
|---|---|
| `health-` | health headlines / labels (`health-uploading = Uploading { $count -> [one] 1 file *[other] { $count } files }`) |
| `panel-` | the panel popup (applet / plasmoid) |
| `details-` | the details window |
| `settings-` | the settings window |
| `offline-` | offline folders: phases, consent, setup steps, keeper errors |
| `migrate-` | the migration: plan, errors, banner labels |
| `share-`, `drive-` | sharing and Drive metadata |
| `cli-` | CLI output |
| `common-` | short words used in several places (`common-open`, `common-cancel`) |
| `cosmic-`, `plasma-`, `dolphin-` | text that genuinely exists in one shell only |

Prefer a shared prefix; a shell prefix needs a reason (a toolkit-only control).

**Rust** (`core`, `view`, `cli`, `cosmic`, `plasma/*`): `rclone_ui_i18n::fl!("id")` /
`fl!("id", count = n)`. The macro wraps `i18n_embed_fl::fl!` with the shared loader, so ids and
argument names are checked at compile time; each crate that calls it has an `i18n.toml` pointing
`assets_dir` at the repo `i18n/` directory (with the shared domain). Language comes from
`DesktopLanguageRequester`, initialised once by `rclone_ui_i18n::init()`.

Strings with arguments or plurals are **always formatted in Rust**. `view` types hand the
toolkits finished strings (`DetailsView`, `SettingsView`, `StatusJson.healthLabel`, ...).

**Kirigami windows** (`plasma/ui`): a cxx-qt singleton `Strings` with `Q_INVOKABLE t(id)` that
calls the same loader. QML writes `Strings.t("details-open-folder")`.

**Plasmoid and Dolphin plugins** (not our process): the daemon exports
`Strings() -> a{ss}`: every argument-free message, resolved for the current locale. The plasmoid
(and its config page) loads it into `property var s` and writes `s["panel-open-folder"]`; until
`Strings()` has answered, `s` is empty (every id reads as `""`, never as the raw id) and the popup
and config page show a busy indicator instead of text, retrying until the daemon answers. The C++
plugins use `DaemonClient::text("dolphin-details")`. Anything with arguments arrives preformatted
in `StatusJson` or another D-Bus reply; a config read failure arrives as `GetConfig`'s
`{"errorText": ...}`.

**One message per meaning.** The COSMIC and Kirigami windows share ids for the same control
(`details-heading-sync-status`, `settings-keep-offline-switch`, `common-open`, ...); a
`cosmic-`/`plasma-`/`dolphin-` id exists only where the control exists in one shell only (the Files
context-menu toggle, the Dolphin section, the plasmoid config page, shell-specific hints). The
temporary per-branch "zone" blocks used while the windows were written are gone; the `.ftl` has
one section per prefix.

**Enforcement** (`crates/i18n/tests/strings_sync.rs`, runs in the `core` CI job, no Qt needed):
1. Every id referenced anywhere exists in `en` (Rust is compile-checked; the test scans QML for
   `Strings.t("…")` / `s["…"]` and C++ for `text("…")`).
2. No id in `en` is unreferenced.
3. Messages exported over D-Bus have no variables.
4. No hard-coded user-visible literals: QML `text:` / `title:` / `placeholderText:` /
   `ToolTip.text:` / `subtitle:` / `label:` with a string literal, iced `text("…")` /
   `button::text("…")` / `.title("…")`, and C++ `i18n("…")` / `QStringLiteral("…")` used as a
   label all fail, unless the line ends with `// i18n-ignore: <reason>` (or `/* … */` in QML
   where needed). `metadata.json` / `.desktop` names are exempt (they have their own
   translation mechanisms).

## CI

`.github/workflows/ci.yml`:
- `changes`: `dorny/paths-filter` decides which jobs matter.
  - `crates/**`, `i18n/**`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.github/**`, `justfile` → everything
  - `cosmic/**` → `cosmic`; `plasma/**` → `plasma`; `resources/**` → `cosmic` + `plasma`
- `fmt` (always), `core` (`-p` the `crates/*` packages: clippy, tests incl. strings sync; no
  system packages), `cosmic` (Wayland/xkb packages; clippy + tests for the COSMIC crate),
  `plasma` (Fedora container with Qt6/KF6 dev packages; clippy + tests for the Plasma crates,
  cmake build of `plasma/dolphin`, `qmllint` on the QML).
- `ci-ok`: one required check that passes when every needed job passed or was skipped.
- `live` (weekly schedule + manual dispatch): latest rclone release, `just test-live`.
- Per-job `Swatinem/rust-cache` keys; caches saved only from `main`.
- `cargo build`/`test` at the root without `-p` builds only `crates/*` (`default-members`).
