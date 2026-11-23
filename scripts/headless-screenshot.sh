#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
#
# Screenshot one window in a throwaway, headless KWin session. Never touches the real desktop:
#
# - KWin runs with its virtual backend (no screen, no input devices) on its own socket;
# - everything runs under `env -i` with a temporary HOME, XDG dirs and XDG_RUNTIME_DIR, inside
#   its own `dbus-run-session`, so the real session bus, display, config, systemd --user manager
#   and rclone config are unreachable (`systemctl --user` simply fails in there);
# - the sandbox is deleted afterwards unless --keep is given.
#
# Usage: scripts/headless-screenshot.sh -o OUT.png [-d SECONDS] [-c CONFIG.toml] [--keep]
#            [--size WxH] -- COMMAND [ARGS...]
#
# The command runs with HOME set to the sandbox's home, which is seeded with CONFIG (default:
# scripts/screenshot-fixture/config.toml) and the folders it names. `~` in the config and
# `@HOME@` in the command's arguments mean the sandbox home. After SECONDS (default 6) the
# active window is captured with Spectacle.

set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
out=""
delay=6
config="$here/screenshot-fixture/config.toml"
keep=0
size="1100x900"

while [[ $# -gt 0 ]]; do
    case "$1" in
    -o) out=$2; shift 2 ;;
    -d) delay=$2; shift 2 ;;
    -c) config=$2; shift 2 ;;
    --size) size=$2; shift 2 ;;
    --keep) keep=1; shift ;;
    --) shift; break ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done
if [[ -z "$out" || $# -eq 0 ]]; then
    sed -n '/^# Usage/,/^# active window/p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
fi
for tool in kwin_wayland spectacle dbus-run-session; do
    command -v "$tool" >/dev/null || { echo "$tool is required (KDE Plasma's kwin and spectacle)" >&2; exit 1; }
done

# Resolve paths before the environment is cleared.
out=$(realpath -m "$out")
config=$(realpath "$config")
cmd=("$@")
if [[ "${cmd[0]}" == */* ]]; then
    cmd[0]=$(realpath "${cmd[0]}")
fi
width=${size%x*}
height=${size#*x}
mkdir -p "$(dirname "$out")"
rm -f "$out"

sandbox=$(mktemp -d "${TMPDIR:-/tmp}/rcse-headless.XXXXXX")
cleanup() {
    if [[ $keep -eq 1 ]]; then
        echo "sandbox kept: $sandbox" >&2
    else
        rm -rf "$sandbox"
    fi
}
trap cleanup EXIT

home="$sandbox/home"
run="$sandbox/run"
for i in "${!cmd[@]}"; do
    cmd[i]=${cmd[i]//@HOME@/$home}
done
mkdir -p "$home/.config/rclone-shell-ext" "$home/.local/state" "$home/.local/share" "$run"
chmod 700 "$run"
cp "$config" "$home/.config/rclone-shell-ext/config.toml"
# Create the folders the fixture refers to (`source = "~/..."` and `mount_path`), so the windows
# show them as they would on a real machine.
grep -oE '"~/[^"]*"' "$config" | tr -d '"' | while read -r p; do
    mkdir -p "$home/${p#\~/}"
done

# The script KWin runs as its session: start the window, wait, capture the active window (or the
# whole virtual screen if nothing is active), then end the session.
cat >"$sandbox/session.sh" <<EOF
#!/usr/bin/env bash
$(printf '%q ' "${cmd[@]}") >"$sandbox/app.log" 2>&1 &
sleep $(printf '%q' "$delay")
spectacle --background --nonotify --activewindow --output $(printf '%q' "$out") >"$sandbox/spectacle.log" 2>&1 || true
if [[ ! -s $(printf '%q' "$out") ]]; then
    spectacle --background --nonotify --fullscreen --output $(printf '%q' "$out") >>"$sandbox/spectacle.log" 2>&1 || true
fi
EOF
chmod +x "$sandbox/session.sh"

env -i \
    PATH="/usr/local/bin:/usr/bin:/bin" \
    LANG="${LANG:-C.UTF-8}" \
    HOME="$home" \
    XDG_CONFIG_HOME="$home/.config" \
    XDG_STATE_HOME="$home/.local/state" \
    XDG_DATA_HOME="$home/.local/share" \
    XDG_CACHE_HOME="$home/.cache" \
    XDG_RUNTIME_DIR="$run" \
    RCLONE_CONFIG=/dev/null \
    RCLONE_SHELL_EXT_NO_MIGRATE=1 \
    timeout 120 dbus-run-session -- \
    kwin_wayland --virtual --width "$width" --height "$height" \
    --socket "rcse-headless-$$" --exit-with-session "$sandbox/session.sh" \
    >"$sandbox/kwin.log" 2>&1 || true

if [[ ! -s "$out" ]]; then
    echo "no screenshot taken; logs:" >&2
    tail -n 20 "$sandbox/app.log" "$sandbox/spectacle.log" "$sandbox/kwin.log" >&2 2>/dev/null || true
    keep=1
    exit 1
fi
echo "$out"
