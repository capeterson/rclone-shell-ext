mod app;
mod config;
mod context_menu;
mod menu;
mod mime_icon;
mod settings;

use config::ConfigExt as _;
use rclone_ui_core::identity::{set_identity, Identity, COSMIC_DESKTOP_ID, DEV_COSMIC_DESKTOP_ID};

// The toolkit-free logic lives in `rclone-ui-core`; re-exported so `crate::rc::...` paths work.
pub use rclone_ui_core::{
    conflict, drive, fuse_conf, history, keeper, offline_setup, pin, rc, share, status, units,
    version,
};

/// Build with `APP_ID_OVERRIDE=com.ca-peterson.rclone-shell-ext-dev` to get a side-by-side dev
/// copy with its own panel identity, config and data (see `just cosmic::install-dev`).
pub const DEV: bool = option_env!("APP_ID_OVERRIDE").is_some();

/// The applet's desktop id, which is also its panel identity.
pub const APP_ID: &str = if DEV {
    DEV_COSMIC_DESKTOP_ID
} else {
    COSMIC_DESKTOP_ID
};

fn main() -> cosmic::iced::Result {
    set_identity(Identity::from_override(option_env!("APP_ID_OVERRIDE")));
    rclone_ui_core::config::install_default_store();
    let args: Vec<String> = std::env::args().skip(1).collect();
    rclone_shell_ext_cli::startup(&args);
    // `--settings` opens the settings window (also reachable from the panel popup).
    if args.iter().any(|a| a == "--settings") {
        return settings::run(config::Config::load());
    }
    // `--menu PATH...` is the "rclone…" window COSMIC Files opens from its right-click menu.
    if let Some(i) = args.iter().position(|a| a == "--menu") {
        return menu::run(config::Config::load(), args[i + 1..].to_vec());
    }
    // `--menu-install` adds it (the applet also does this on start-up).
    if args.iter().any(|a| a == "--menu-install") {
        context_menu::install();
        return Ok(());
    }
    // `--menu-remove` takes the applet's entry back out of the COSMIC Files right-click menu.
    if args.iter().any(|a| a == "--menu-remove") {
        context_menu::remove();
        return Ok(());
    }
    // Everything else the shared command line does (`--status`, `--share-url`, `--pin`,
    // `--migrate`, ...) is the same code the `rclone-shell-ext` binary runs.
    if let Some(code) = rclone_shell_ext_cli::run(&args) {
        std::process::exit(code);
    }
    cosmic::applet::run::<app::AppModel>(())
}
