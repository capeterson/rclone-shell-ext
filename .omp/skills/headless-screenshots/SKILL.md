---
name: headless-screenshots
description: Use to see what a COSMIC or Plasma window of rclone-shell-ext actually looks like (settings, details, any UI change) by screenshotting it in a sandboxed headless KWin session, never the user's real desktop.
---

# Headless screenshots for rclone-shell-ext

Visual proof for UI changes without touching the user's desktop. A throwaway KWin runs with its
virtual backend (no screen, no input) inside `env -i` + `dbus-run-session`, with a temporary HOME,
XDG dirs and `XDG_RUNTIME_DIR`; Spectacle captures the active window there. Needs only what a
Plasma install already has (`kwin_wayland`, `spectacle`, `dbus-run-session`); no X11, no Xvfb.

## Commands

```sh
just screenshots                         # both settings windows -> target/screenshots/*.png
just screenshot NAME CMD [ARGS...]       # any window -> target/screenshots/NAME.png
OPTS="-d 10 --size 1280x1000" just screenshot NAME CMD ...   # longer wait, bigger screen
OPTS="-c my-fixture.toml --keep" just screenshot NAME CMD ... # own settings; keep sandbox + logs
```

- Build first (`cargo build -p rclone-shell-ext-cosmic -p rclone-shell-ext-plasma`); pass
  binaries as `target/debug/...` paths. Examples: `target/debug/rclone-shell-ext-cosmic --settings`,
  `target/debug/rclone-shell-ext-plasma --settings`,
  `target/debug/rclone-shell-ext-plasma --menu @HOME@/GoogleDrive/report.pdf`. `@HOME@` in the
  arguments becomes the sandbox home; don't use `~` there (your shell would expand it to yours).
- The default fixture is `scripts/screenshot-fixture/config.toml` (one backed-up folder, mount
  not running). For other states copy it, edit, and pass `-c`. Folders written as `"~/..."` in the
  fixture are created inside the sandbox home.
- **Keeper state** (`backup.json`, `offline.json` in `$XDG_STATE_HOME/rclone-shell-ext/`): wrap
  the command in a small script that writes the files and then `exec`s the window; it runs inside
  the sandbox, so `$HOME`/`$XDG_STATE_HOME` are the sandbox's. Status keys must be the
  `source`/`rel` strings exactly as in the fixture config (`"~/Documents"`, not the expanded
  path), or every row falls back to its default state. To show a pass as *running*, also hold
  the run lock: `flock "$st/backup-running.lock" sleep 60 &` before `exec`.
- COSMIC icons: the script points libcosmic at `breeze-dark` in the sandbox when the Cosmic icon
  theme isn't installed; without that every COSMIC icon renders blank. Blank icons elsewhere are
  also a sandbox theme issue, not proof the code lacks them.
- Look at the result with the `read` tool on the PNG. Report what you saw, and that it is a
  software-rendered headless KWin (layout and text, not performance or the real theme's exact
  look; the COSMIC window gets KWin's decorations, not COSMIC's).
- On failure the script keeps the sandbox and prints the app, Spectacle and KWin logs.

## Never touch the real desktop

- Only capture through `scripts/headless-screenshot.sh`. Never run `spectacle`, `grim`,
  `import`, `gnome-screenshot`, KWin's ScreenShot2 D-Bus API, or any capture tool in the user's
  own session, and never ask a portal to screenshot.
- Never pass through `WAYLAND_DISPLAY`, `DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`, the real
  `XDG_RUNTIME_DIR` or the real `HOME` into the sandbox, and never remove `env -i` or
  `dbus-run-session` from the script. That isolation is what keeps windows, notifications,
  `systemctl --user`, migrations and config writes away from the user's session.
- Never kill compositor or shell processes by name (`pkill kwin_wayland`, `pkill plasmashell`,
  `killall spectacle`): that kills the user's desktop. The sandboxed KWin exits by itself when
  its session script ends, and `timeout 120` bounds it.
- Never point a fixture at real paths (`/home/<user>/...`) or real remotes. `RCLONE_CONFIG` is
  `/dev/null` in the sandbox, so rclone has no remotes; keep it that way.
- Don't launch the installed binaries from `~/.local/bin` for screenshots; use the build in
  `target/` so what you look at is the code you changed.
- Don't commit screenshots; `target/` is ignored.

## Limits

- Startup only: no clicks or typing. For a state behind a click, prefer seeding config and
  state (above) over scripting input; say plainly which states you couldn't reach.
- The COSMIC panel applet (layer-shell inside cosmic-comp) and Dolphin's context menu can't be
  shown this way.
