// SPDX-License-Identifier: GPL-3.0-only
//! Kirigami windows for rclone-shell-ext on Plasma, hosted from Rust with cxx-qt. The binary crate calls these
//! entry points after it has installed the config store and identity
//! (`rclone_ui_core::config::install_default_store`, `rclone_ui_core::identity::set_identity`). Each runs a Qt
//! event loop and returns an exit code.

mod bridge;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};

/// The QML module URI (see `build.rs`).
const MODULE_URI: &str = "com.ca_peterson.RcloneShellExt";
const APP_ID: &str = rclone_ui_core::identity::PLASMA_DESKTOP_ID;

/// `rclone-shell-ext-plasma --menu PATH...`: the details window for a mounted file or folder.
pub fn run_details(paths: &[String]) -> i32 {
    bridge::details::set_initial_paths(paths);
    run_qml("Details.qml", "rclone")
}

/// `rclone-shell-ext-plasma --settings`: the settings window (offline folders setup, integrations).
pub fn run_settings() -> i32 {
    run_qml("Settings.qml", "rclone settings")
}

/// Create the application and engine, load one window from the QML module, and run the event
/// loop until the window closes.
fn run_qml(file: &str, display_name: &str) -> i32 {
    // Pick the user's languages before any QML asks for text.
    rclone_ui_i18n::init();
    // Make sure the generated QML types and the module's resources are linked in.
    cxx_qt::init_crate!(rclone_shell_ext_plasma_ui);
    cxx_qt::init_qml_module!("com.ca_peterson.RcloneShellExt");

    // Kirigami wants the desktop style; a user's own QT_QUICK_CONTROLS_STYLE still wins.
    if std::env::var_os("QT_QUICK_CONTROLS_STYLE").is_none() {
        QQuickStyle::set_style(&QString::from("org.kde.desktop"));
    }

    let mut app = QGuiApplication::new();
    let Some(mut app) = app.as_mut() else {
        eprintln!("rclone-shell-ext-plasma: could not create the Qt application");
        return 1;
    };
    app.as_mut()
        .set_organization_name(&QString::from("ca-peterson"));
    app.as_mut()
        .set_application_name(&QString::from("rclone-shell-ext"));
    app.as_mut()
        .set_application_display_name(&QString::from(display_name));
    // Plasma matches the window to its launcher (and takes the `com.ca-peterson.rclone-shell-ext` icon) through
    // the desktop file name.
    QGuiApplication::set_desktop_file_name(&QString::from(APP_ID));
    // TODO: cxx-qt-lib has no QIcon, so the window icon (`com.ca-peterson.rclone-shell-ext`) is not set
    // explicitly; it comes from the installed desktop file / icon theme.

    let mut engine = QQmlApplicationEngine::new();
    let Some(mut engine) = engine.as_mut() else {
        eprintln!("rclone-shell-ext-plasma: could not create the QML engine");
        return 1;
    };
    let url = qml_url(file);
    engine.as_mut().load(&url);
    app.exec()
}

/// Where a file of the QML module lives inside the compiled-in resources.
fn qml_url(file: &str) -> QUrl {
    QUrl::from(qml_url_string(file).as_str())
}

fn qml_url_string(file: &str) -> String {
    format!("qrc:/qt/qml/{}/qml/{file}", MODULE_URI.replace('.', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qml_files_resolve_inside_the_module() {
        assert_eq!(
            qml_url_string("Details.qml"),
            "qrc:/qt/qml/com/ca_peterson/RcloneShellExt/qml/Details.qml"
        );
    }
}
