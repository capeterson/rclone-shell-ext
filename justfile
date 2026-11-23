mod cosmic 'cosmic/justfile'
mod plasma 'plasma/justfile'

default:
    @just --list

# Everything CI runs, for the parts you have toolchains for
ci: core
    just cosmic::ci
    just plasma::ci

# Toolkit-free crates (CI `fmt` + `core` jobs): format, lint, test
core:
    cargo fmt --all --check
    cargo clippy -p rclone-ui-core -p rclone-ui-i18n -p rclone-ui-view -p rclone-shell-ext --all-targets --locked -- -D warnings
    cargo test -p rclone-ui-core -p rclone-ui-i18n -p rclone-ui-view -p rclone-shell-ext --locked

# Tests that run against the installed rclone, to catch behaviour that changes between its
# releases (start a throwaway `rclone rcd` for them)
test-live:
    #!/usr/bin/env bash
    set -euo pipefail
    dir=$(mktemp -d /tmp/rcs-it.XXXXXX)
    rclone rcd --rc-no-auth --rc-addr 127.0.0.1:5611 --config /dev/null --log-level NOTICE >"$dir/rcd.log" 2>&1 &
    pid=$!
    trap 'kill $pid 2>/dev/null; rm -rf "$dir"' EXIT
    sleep 2
    RCS_IT_DIR="$dir" cargo test -p rclone-ui-core --locked -- --ignored real_rclone

# Screenshot any window in a throwaway headless KWin (never your desktop), e.g.
# `just screenshot cosmic-settings target/debug/rclone-shell-ext-cosmic --settings`.
# Writes target/screenshots/NAME.png. Options for scripts/headless-screenshot.sh go in OPTS.
screenshot name *cmd:
    scripts/headless-screenshot.sh {{env_var_or_default('OPTS', '')}} -o target/screenshots/{{name}}.png -- {{cmd}}

# Screenshot both settings windows (needs the COSMIC and Plasma toolchains)
screenshots:
    cargo build -p rclone-shell-ext-cosmic -p rclone-shell-ext-plasma -p rclone-shell-ext
    just screenshot cosmic-settings target/debug/rclone-shell-ext-cosmic --settings
    just screenshot plasma-settings target/debug/rclone-shell-ext-plasma --settings
