// SPDX-License-Identifier: GPL-3.0-only
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--menu") {
        std::process::exit(rclone_plasma_ui::run_details(&args[i + 1..]));
    }
    if args.iter().any(|a| a == "--settings") {
        std::process::exit(rclone_plasma_ui::run_settings());
    }
}
