// SPDX-License-Identifier: GPL-3.0-only
use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    // The QML files are compiled into the library and registered as the module
    // `com.ca_peterson.RcloneShellExt` (loaded from `qrc:/qt/qml/com/ca_peterson/RcloneShellExt/`).
    CxxQtBuilder::new_qml_module(
        QmlModule::new("com.ca_peterson.RcloneShellExt")
            .qml_files(["qml/Details.qml", "qml/Settings.qml"]),
    )
    .qt_module("Gui")
    .files(["src/bridge/details.rs", "src/bridge/settings.rs"])
    .build();
}
