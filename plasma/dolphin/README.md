# rclone-plasma Dolphin plugins

Two KF6 plugins that talk to the rclone-plasma daemon over the session bus
(see `../docs/dbus-interface.md`):

- `rcloneoverlayplugin` (`kf6/overlayicon`): sync-state emblems on files in the mount.
- `rclonefileitemaction` (`kf6/kfileitemaction`): "rclone" context submenu
  (Details, Keep offline / Stop keeping offline, Manage Sharing).

Both are non-blocking: all D-Bus traffic is asynchronous, and after a failed call
the client stays quiet for 10 s.

## Build

Requires Qt 6, KF6 (KIO, CoreAddons, I18n) and extra-cmake-modules, e.g. on Fedora:
`sudo dnf install cmake gcc-c++ extra-cmake-modules qt6-qtbase-devel kf6-kio-devel kf6-kcoreaddons-devel kf6-ki18n-devel`

    cmake -B build -S . -DCMAKE_INSTALL_PREFIX=/usr
    cmake --build build
    sudo cmake --install build

Plugins install to `<prefix>/lib64/qt6/plugins/kf6/{overlayicon,kfileitemaction}`.
Restart Dolphin (`kquitapp6 dolphin`) afterwards. The `rclone-plasma` binary must
be on `PATH`, and the icons `com.ca-peterson.rclone-shell-ext` and `rclone-shell-ext-emblem-*` installed.
