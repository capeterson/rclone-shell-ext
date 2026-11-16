// SPDX-License-Identifier: GPL-3.0-only
//! The cxx-qt `QObject`s the QML windows bind to.

pub mod details;
pub mod settings;
#[allow(clippy::unnecessary_box_returns)] // cxx-qt boxes the (empty) Rust struct
pub mod strings;
