# rclone-shell-ext Dolphin plugins

Two KF6 plugins that talk to the rclone-shell-ext-plasma daemon over the session bus
(see `../docs/dbus-interface.md`):

- `rcloneoverlayplugin` (`kf6/overlayicon`): sync-state emblems on files in the mount.
- `rclonefileitemaction` (`kf6/kfileitemaction`): "rclone" context submenu. In the mount:
  Details, Keep offline / Stop keeping offline, Manage Sharing. On a single local folder
  outside the mount: Back up to Drive, or Stop backing up if that folder is backed up.

The overlay plugin never blocks: its D-Bus traffic is asynchronous. The action plugin must
answer as the menu opens, so it asks the daemon with blocking calls of at most 300 ms. After a
failed call, either client stays quiet for 10 s.

## Build

Requires Qt 6, KF6 (KIO, CoreAddons) and extra-cmake-modules, e.g. on Fedora:
`sudo dnf install cmake gcc-c++ extra-cmake-modules qt6-qtbase-devel kf6-kio-devel kf6-kcoreaddons-devel`

    cmake -B build -S . -DCMAKE_INSTALL_PREFIX=/usr
    cmake --build build
    sudo cmake --install build

Plugins install to `<prefix>/lib64/qt6/plugins/kf6/{overlayicon,kfileitemaction}`.
Restart Dolphin (`kquitapp6 dolphin`) afterwards. The `rclone-shell-ext-plasma` binary must
be on `PATH`, and the icons `com.ca-peterson.rclone-shell-ext` and `rclone-shell-ext-emblem-*` installed.
