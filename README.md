# rclone-shell-ext

rclone status, offline folders and sharing for Linux desktop shells. It watches an
[rclone](https://rclone.org) mount (synced, uploading, downloading, both, error, or mount down)
and adds Google-Drive-style **offline folders**: a folder you choose is fully downloaded, kept
in sync both ways with `rclone bisync`, and works without a connection, right where it is in the
mount. One shared Rust integration does the rclone, systemd and state work; a thin front end
per shell shows it: a **COSMIC** panel applet with its own windows, and a **KDE Plasma**
plasmoid with Kirigami windows and Dolphin sync emblems and context menu. Both shells use one
identity, so they share one config file and one set of offline folders on a machine.

Status is read from rclone's remote-control API (`--rc`: `core/stats`, `vfs/stats` and
`core/transferred`) plus `systemctl --user` and `/proc/mounts`.

**COSMIC applet:** click the panel icon for details, per-file transfer progress, the 10 most
recently synced files, and Open folder / Restart / Stop buttons. "Show all" opens the full
history (up to 100 files). Clicking a file opens its details window. Files get the same type
icons as COSMIC Files.

**Plasma:** the plasmoid shows the same status and history; Dolphin shows a sync emblem on every
file in the Drive folder and adds an **rclone** action for details, offline folders and sharing.

## Repository layout

A monorepo; read `docs/architecture.md` for how the pieces fit and `AGENTS.md` for the working
rules.

- `crates/core`: toolkit-free rclone logic shared by every front end (rc client, status, config,
  offline folders, units, sharing, migration).
- `crates/i18n`: the UI text (Fluent) and its loader; `i18n/en/rclone_shell_ext.ftl` is the only
  copy of every user-visible string.
- `crates/view`: what a window or panel shows and which buttons apply, as plain Rust.
- `crates/cli`: the shared `rclone-shell-ext` binary (offline-folder keeper, pins, status, sharing, migration).
- `cosmic/`: the COSMIC panel applet (`rclone-shell-ext-cosmic`).
- `plasma/`: the KDE Plasma front end (`daemon/` D-Bus service, `ui/` Kirigami windows,
  `plasmoid/`, `dolphin/` plugins; the D-Bus contract is `plasma/docs/dbus-interface.md`).
- `resources/`: the only copy of the SVG assets (`emblems/` status icons, `overlay/` Dolphin emblems, `icons/` app icons, `loading/`).

Run `just --list` for the build, test and install recipes (`just cosmic::...`, `just plasma::...`).

## Requirements

- **rclone 1.75.0 or newer** (minimum supported version; last verified with 1.75.2). The popup
  shows a warning when the running rclone is older. We raise this minimum whenever the app starts
  relying on a newer rclone feature, so check it when upgrading the app.
- rclone mounted by a systemd **user** service with the rc API enabled and authenticated, e.g.
  `rclone mount remote: ~/Drive --rc --rc-addr 127.0.0.1:5572` with `RCLONE_RC_USER` / `RCLONE_RC_PASS`
  provided through an `EnvironmentFile`. Point the app at it with the settings below.
- A Rust toolchain and [`just`](https://github.com/casey/just) (`cargo install just`).
- For COSMIC: the COSMIC desktop and libcosmic's system dev libraries
  (`libxkbcommon-dev libfontconfig-dev libfreetype-dev libexpat1-dev libudev-dev libinput-dev libegl-dev libgbm-dev wayland-protocols`).
- For Plasma: KDE Plasma 6 with Qt 6 and KDE Frameworks 6 development packages (Kirigami,
  KIO, for the Dolphin plugins), `cmake`, and `kpackagetool6`.

## Install

Pick the shell you use; installing both is fine (they share the binary, config and state).

```
just cosmic::install-user   # COSMIC applet: then add "rclone UI" in Settings -> Desktop -> Panel -> Configure panel applets
just plasma::install-user   # Plasma: plasmoid, D-Bus service, Kirigami windows, Dolphin plugins
```

Everything goes under `~/.local`. After a Plasma install, restart Dolphin with `QT_PLUGIN_PATH`
set to `~/.local/lib64/qt6/plugins` (Plasma sets it from the next login).

The Plasma widget is a standalone panel widget; add “rclone UI” through Plasma's widget
explorer. It is not a system-tray item. If you previously used it in the tray, add it to the
panel after upgrading.

**One-time migration.** Both recipes run `rclone-shell-ext --migrate` in the foreground. If you
used the earlier per-shell versions (`cosmic-rclone-ui`, `rclone-plasma`) it moves that install
to the shared identity: it stops the old offline-folder services, moves the local offline copies
and state, renames bisync's working files so the sync baseline survives (no `--resync`), rewrites
the units and drop-in under the new names and restarts what was enabled, and updates the COSMIC
panel entry. It waits until rclone has no running sync jobs and no offline folder is mounted, and
never deletes your data. If anything is unexpected it changes nothing, prints why, and the recipe
stops with the old install still working. Preview it with `rclone-shell-ext --migrate --dry-run`.
Only after it succeeds does the recipe remove the old install's files.
The Plasma recipe also renames the old widget in your panel or system tray, which `--migrate`
does not do: plasmashell keeps the layout in memory and would write it back over a file edited
under it, so the recipe stops plasmashell, runs `rclone-shell-ext --plasma-panel-rename`
(`--dry-run` previews it), and starts plasmashell again. The rename refuses to run while
plasmashell is up. You will see the panel flicker once. If you skip the recipe
and just start a new binary, the daemon or applet does the move in the background and shows a
banner meanwhile; offline folders can't be turned on or pinned until it is done.

**Uninstall:** `just cosmic::uninstall-user` / `just plasma::uninstall-user`. They remove the
offline-folder services first (refused while folders are still kept offline: remove them in
Settings), then the files they installed. The shared binary stays while the other shell is
installed. Your config and the data/state directories are left alone.

Side-by-side dev copy for COSMIC: `just cosmic::install-dev` / `just cosmic::uninstall-dev`
("rclone UI (dev)"; it never migrates and keeps its own config, data and units).

## Configuration

Defaults (stored in `~/.config/rclone-shell-ext/config.toml`, shared by both front ends):

| Key | Default |
|---|---|
| `rc_addr` | `127.0.0.1:5572` |
| `env_file` | `~/.config/rclone/gdrive-rc.env` |
| `service` | `rclone-gdrive.service` |
| `mount_path` | `~/GoogleDrive` |
| `cache_dir` | `~/.cache/rclone` (rclone's `--cache-dir`; read for per-file cache status) |
| `context_menu` | `true` (add the "rclone…" entry to COSMIC Files' right-click menu) |
| `offline_enabled` | `false` (offline folders; switch on in Settings) |
| `pins` | `[]` (folders kept offline, relative to the mount) |
| `sync_interval_secs` | `300` (how often offline folders sync when nothing changes locally) |

The settings window (`--settings`, or the gear in the popup) covers offline folders and the
right-click menu entry. Plasma's plasmoid configuration page edits the connection settings
(`rc_addr`, `env_file`, `service`, `mount_path`, `cache_dir`, `sync_interval_secs`).

## Offline folders

Like Google Drive's "Available offline": a pinned folder is fully downloaded, kept in sync both
ways, and works with no connection, right where it is in the Drive folder. Turn it on in
Settings, then right-click a folder in COSMIC Files → **rclone…** → **Keep offline**, or in
Dolphin → **rclone** → **Keep offline**.

How it works:

- The folder's local copy lives in `~/.local/share/rclone-shell-ext/offline/<id>` and is
  synced with `rclone bisync`, run *inside the mount's own rclone process* through its rc API
  (`sync/bisync`), so there is no second rclone config or login. A background user service
  (`rclone-shell-ext-offline.service`) runs each sync: every `sync_interval_secs`, and about
  10 seconds after a local change.
- The local copy is mounted on top of the folder's place in the Drive folder
  (`rclone-shell-ext-pin@<id>.service`), so it shows up in place. Both services are bound to
  the mount's service and start and stop with it; a folder that has been downloaded is mounted
  even when offline.
- Conflicts (the same file changed on both sides between syncs) are resolved as *newer wins*:
  the older edit is uploaded first, so it stays in the file's version history in Google Drive
  ("Manage versions"; Drive keeps old versions of uploaded files for about 30 days or 100
  versions unless one is marked "Keep forever"). You get a notification.
- Renaming or moving a file or folder (on either side) is a move in the cloud, not a second
  upload, and a sync that is cut off (crash, reboot, lost connection) recovers by itself on the
  next run.
- Native Google Docs, Sheets and Slides are left out of the offline copy (rclone can only
  download exports of them, which can't be synced back); they still open from the main mount.
- If bisync stops for safety (too many deletions, or the folder vanished in the cloud), the
  folder is marked **Needs attention**; **Repair** in Settings merges both sides without deleting
  anything (when a file differs on both sides, the newer one is kept).
- Unpinning uploads the last changes first (it waits if you're offline), then unmounts and deletes
  the local copy.

Turning the switch on first shows a dialog listing every change setup will make (only the ones
not already done); **Continue** goes on to the password prompt, **Cancel** changes nothing and
leaves the switch off. If setup can't finish (for example the password prompt is dismissed), it
is undone and the switch goes back off. Before `/etc/fuse.conf` is edited it is copied to
`/etc/fuse.conf.rclone-shell-ext-<date-time>.bak`; if that copy fails the file is left alone.

What setup does: mounting inside the Drive mount needs the Drive mount to use
`--allow-other`, which ordinary users can only request once `user_allow_other` is set in
`/etc/fuse.conf` (one line, added through the system password prompt). The app then adds a
drop-in for the mount's service (`<service>.d/50-rclone-shell-ext.conf`) that sets
`--allow-other` together with `--default-permissions` and a `077` umask, so other users still
can't open your files, and restarts the mount once. Turning offline folders off removes the
services and the drop-in; the `/etc/fuse.conf` line is left in place.

Each file a sync changes shows in the popup's recent activity with two small icons (hover for a
tooltip): what happened (new, changed or deleted) and which way (an up arrow for this computer
→ cloud, a down arrow for cloud → here). The keeper works this out by comparing the local copy
with bisync's listings before and after each sync. A sync that would delete more than half of a
folder's files (and more than 3) stops with **Needs attention** instead.

See [Known limits](#known-limits) for what offline folders can't do.

Command-line control of offline folders: see [Command line](#command-line).

## Details window

Right-click a file or folder in COSMIC Files and choose **rclone…** (in Dolphin: **rclone** →
**Details…**) (the applet adds this entry
to COSMIC Files' `context_actions` config when it starts; COSMIC Files cannot nest custom
actions in a submenu, so it is one entry that opens this window). Clicking an item in the
applet's recent-files list opens the same window. It shows:

- **Sync status**: for a file, whether it is in sync, online only, uploading, queued, downloading,
  partly downloaded, or differs from Google Drive, with Drive's modified time, local cache state,
  size and version; for a folder, counts of files uploading, queued and downloading and how much
  is cached. Local state comes from rclone's rc API and its VFS cache metadata on disk.
- **Sharing**: whether the item is shared and a table of who has access (name, role, type).
- **Open** (a file expands to *Folder* / *File*; a folder opens directly) and **Manage Sharing**,
  which opens the item in Google Drive with the sharing dialog requested.

Google Drive data comes from the Drive API using the OAuth token rclone already holds: the
applet asks rclone's rc API for the current token, so it never reads `rclone.conf` or stores
credentials. Only Google Drive remotes are supported for now. `Manage Sharing` relies on the
`sharingaction=manageaccess` URL parameter, which Google does not document; if it stops working
the item still opens in Drive, just without the dialog.

Command-line helpers: `--menu PATH` opens the window, `--share-url PATH` prints the Manage
Sharing link, and `--menu-install` / `--menu-remove` add or remove the COSMIC Files entry.
The Plasma build shows the same content in a Kirigami window.

## Command line

The shared `rclone-shell-ext` binary (`rclone-shell-ext --help`, or any unrecognised mode, prints this usage and exits with status 2):

| Mode | Does |
|---|---|
| `--status [--rc-addr HOST:PORT]` | prints one status snapshot without any panel |
| `--share-url PATH` / `--share-open PATH` | prints / opens the Manage Sharing link for an item in the mount |
| `--pin PATH` / `--unpin PATH` / `--pins` | keep a folder offline, stop doing so, list offline folders |
| `--offline-repair FOLDER` | ask the keeper to rebuild a folder's sync baseline |
| `--offline-remove` | remove the offline-folder services (refuses while folders are pinned) |
| `--migrate [--dry-run]` | move an old `cosmic-rclone-ui` / `rclone-plasma` install over |
| `--plasma-panel-rename [--dry-run]` | rename the old plasmoid in plasmashell's applet list (refuses while plasmashell runs) |
| `--offline-keeper`, `--pin-mount ID`, `--pin-unmount ID` | run by the systemd units; not for typing |

The front-end binaries add their own modes (`rclone-shell-ext-plasma --daemon`, `--menu PATH`,
`--settings`, `--status --json`; `rclone-shell-ext-cosmic --settings`, `--menu PATH`, ...).

## Development

```
just ci            # everything CI runs, for the parts you have toolchains for
just core::ci      # or just cosmic::ci, just plasma::ci
just test-live     # the tests that drive a throwaway `rclone rcd` (run after upgrading rclone)
just screenshots   # both settings windows, rendered in a sandboxed headless KWin -> target/screenshots/
just screenshot NAME CMD...   # any window the same way (see scripts/headless-screenshot.sh)
cargo test --workspace --locked
```

- Tests are hermetic: they use temp directories and injected roots, never the real `$HOME`,
  and a test run never runs the migration.
- Screenshots never touch your desktop: `scripts/headless-screenshot.sh` runs a virtual-backend
  `kwin_wayland` under `env -i` and `dbus-run-session` with a throwaway HOME, XDG dirs and
  runtime dir (sample settings from `scripts/screenshot-fixture/`), and captures with Spectacle
  inside it. Needs `kwin_wayland` and `spectacle` (any Plasma install); no X11.
- UI text: every user-visible string lives in `i18n/en/rclone_shell_ext.ftl`; add or change
  wording there and run `cargo test -p rclone-ui-i18n` (the sync check). See `AGENTS.md`
  ("UI text") for the id and prefix rules.
- Before changing the Plasma D-Bus contract, edit `plasma/docs/dbus-interface.md` first.
- Clippy is pedantic with `-D warnings`; keep `cargo fmt` clean; keep `Cargo.lock` committed.

## Notes

A running transfer counts as a download when rclone reports a source but no destination (a read
through the mount), and as an upload when it has both (a write-back from the VFS cache, or a sync).

The recent-files history comes from rclone's in-memory list of completed transfers, so it
starts empty and resets when rclone restarts. Repeats of the same file are shown as one row
(`×N`) at its latest time, and new entries appear after a 2 second delay so the list doesn't
jump on every poll. rclone reports each transfer's source and destination, so rows get an upload or download arrow.
File icons are chosen from the file name alone (shared-mime-info plus your icon theme); files
are never read, so nothing is downloaded to draw the list.

## Known limits

- **New / changed / deleted icons only show for offline folders.** rclone's transfer history
  doesn't say whether a file was new or changed, and keeps no record of deletions at all (they are
  only logged at debug level). So for the rest of the Drive folder, rows have no kind icon, and
  files deleted there (or on another device) don't appear.
- **Offline folders are folders only**, can't be nested inside each other, and syncing is by the
  folder's Google Drive id. A folder that is renamed or moved in Drive needs to be taken offline
  again, and one deleted and recreated under the same name stops with **Needs attention** (Repair
  merges both sides). Only non-Drive remotes sync by path, and those refuse very long folder paths
  (rclone's bisync names its working files after both paths).
- **Renaming a folder that holds more than half of an offline folder's files** looks like mass
  deletion to bisync, even with rename tracking, so it stops with **Needs attention**; Repair
  fixes it. (`--max-delete-renames-aware` arrived after rclone 1.75.)
- **Moving a file into or out of an offline folder** is a copy plus a delete, because it crosses
  mounts.
- **Deleting everything in an offline folder** (either side) is refused by bisync as an unsafe
  empty listing; Repair brings the files back.
- Files named `*.partial` are never synced in offline folders (rclone's temporary files).

## Tests against rclone

`just test-live` runs the tests that drive the installed rclone through its rc API (renames,
conflicts, delete limits, failure messages). Run it after upgrading rclone, and before raising
the minimum version above.

## License

GPL-3.0-only
