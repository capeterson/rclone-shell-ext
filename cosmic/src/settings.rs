// SPDX-License-Identifier: GPL-3.0-only
//! The settings window (`--settings`, or "Settings…" in the panel popup).
//!
//! Its main job is offline folders. What the window says and the consent -> setup -> rollback
//! flow live in `rclone_ui_view::settings` (shared with the Plasma window); this file is the
//! iced widgets, messages and worker tasks around it.

use crate::config::{Config, ConfigExt, ConfigWatch, WATCH_EVERY};
use crate::offline_setup::{set_up, tear_down, turn_on, Checks};
use crate::pin::{self, OfflineState};
use crate::{keeper, share};
use cosmic::app::Settings;
use cosmic::iced::{time, Alignment, Length, Size, Subscription};
use cosmic::prelude::*;
use cosmic::widget::{self, settings};
use rclone_ui_i18n::fl;
use rclone_ui_view::settings::{ConsentAction, SettingsState, ToggleAction};
use rclone_ui_view::REFRESH;

#[derive(Debug, Clone)]
enum Message {
    Refresh,
    Refreshed(Checks, OfflineState),
    /// Time to look at the settings file for outside changes.
    ConfigTick,
    ToggleOffline(bool),
    SetUp,
    /// The consent dialog's answer.
    Consent(bool),
    /// A setup or teardown finished, with a note to show on success.
    Done(Result<Option<String>, String>),
    Unpin(String),
    Repair(String),
    Open(String),
    ToggleContextMenu(bool),
}

struct App {
    core: cosmic::Core,
    st: SettingsState,
    config_watch: ConfigWatch,
}

type Task = cosmic::app::Task<Message>;

fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
    msg: impl Fn(T) -> Message + Send + 'static,
) -> Task {
    Task::perform(
        async move { tokio::task::spawn_blocking(f).await },
        move |r| match r {
            Ok(v) => cosmic::Action::App(msg(v)),
            Err(e) => cosmic::Action::App(Message::Done(Err(e.to_string()))),
        },
    )
}

impl cosmic::Application for App {
    type Executor = cosmic::executor::Default;
    type Flags = Config;
    type Message = Message;
    const APP_ID: &'static str = crate::APP_ID;

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(mut core: cosmic::Core, cfg: Config) -> (Self, Task) {
        core.window.show_maximize = false;
        core.set_header_title(fl!("settings-window-title"));
        let app = App {
            core,
            st: SettingsState::new(cfg),
            config_watch: ConfigWatch::default(),
        };
        (app, Task::done(cosmic::Action::App(Message::Refresh)))
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            time::every(REFRESH).map(|_| Message::Refresh),
            time::every(WATCH_EVERY).map(|_| Message::ConfigTick),
        ])
    }

    fn update(&mut self, message: Message) -> Task {
        match message {
            Message::Refresh => {
                let cfg = self.st.cfg.clone();
                return blocking(
                    move || (Checks::probe(&cfg), OfflineState::load()),
                    |(c, s)| Message::Refreshed(c, s),
                );
            }
            Message::Refreshed(c, s) => {
                self.st.checks = c;
                self.st.state = s;
            }
            Message::ConfigTick => {
                if let Some(c) = self.config_watch.poll() {
                    self.st.cfg = c;
                }
            }
            Message::ToggleOffline(on) => {
                if self.st.toggle_offline(on) == ToggleAction::TearDown {
                    let cfg = self.st.cfg.clone();
                    return blocking(move || tear_down(&cfg), Message::Done);
                }
            }
            Message::SetUp => self.st.request_setup(),
            Message::Consent(go) => {
                let action = self.st.answer_consent(go);
                let cfg = self.st.cfg.clone();
                return match action {
                    ConsentAction::Cancel => Task::none(),
                    ConsentAction::SetUp => blocking(move || set_up(&cfg), Message::Done),
                    ConsentAction::TurnOn => blocking(move || turn_on(&cfg), Message::Done),
                };
            }
            Message::Done(r) => {
                // A failed turn-on switches itself back off; show that now, not on the next
                // config notification.
                self.st.finish(r, Config::load());
                return self.update(Message::Refresh);
            }
            Message::Unpin(rel) => {
                if let Err(e) = Config::save(|c| c.pins.retain(|p| *p != rel)) {
                    self.st.error = Some(e);
                }
            }
            Message::Repair(rel) => {
                if let Err(e) = keeper::request_repair(&rel) {
                    self.st.error = Some(e.to_string());
                }
            }
            Message::Open(rel) => {
                share::open_url(&pin::mount_point(&self.st.cfg, &rel).to_string_lossy());
            }
            Message::ToggleContextMenu(on) => match Config::save(|c| c.context_menu = on) {
                Ok(_) if on => crate::context_menu::install(),
                Ok(_) => crate::context_menu::remove(),
                Err(e) => self.st.error = Some(e),
            },
        }
        Task::none()
    }

    /// Informed consent before setup changes anything: what will change, then Continue (on to
    /// the password prompt) or Cancel (nothing is changed and the switch stays off).
    fn dialog(&self) -> Option<Element<'_, Message>> {
        if !self.st.consent {
            return None;
        }
        let list = self.st.consent_items().into_iter().fold(
            widget::column::with_capacity(8).spacing(cosmic::theme::spacing().space_s),
            |col, (title, detail)| {
                col.push(
                    widget::column::with_capacity(2)
                        .spacing(2)
                        .push(widget::text::heading(title))
                        .push(widget::text::body(detail)),
                )
            },
        );
        Some(
            widget::dialog()
                .title(fl!("settings-consent-title"))
                .icon(widget::icon::from_name("folder-download-symbolic").size(64))
                .body(fl!("settings-consent-intro"))
                .control(widget::container(widget::scrollable(list)).max_height(360.0))
                .primary_action(
                    widget::button::suggested(self.st.consent_go_label())
                        .on_press(Message::Consent(true)),
                )
                .secondary_action(
                    widget::button::standard(fl!("common-cancel"))
                        .on_press(Message::Consent(false)),
                )
                .into(),
        )
    }

    fn view(&self) -> Element<'_, Message> {
        let now = jiff::Timestamp::now().as_second();
        let mut sections: Vec<Element<'_, Message>> = vec![self.offline_section(now)];
        let notices = self.st.notices(now);
        if !notices.is_empty() {
            sections.push(Self::notices_section(notices));
        }
        sections.push(
            settings::section()
                .title(fl!("settings-heading-general"))
                .add(
                    // The one COSMIC-only control: COSMIC Files has an extension point of its own.
                    settings::item::builder(fl!("cosmic-settings-context-menu"))
                        .toggler(self.st.cfg.context_menu, Message::ToggleContextMenu),
                )
                .into(),
        );
        for (icon, text) in [
            ("dialog-information-symbolic", &self.st.note),
            ("dialog-error-symbolic", &self.st.error),
        ] {
            if let Some(t) = text {
                sections.push(
                    widget::row::with_capacity(2)
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .push(widget::icon::from_name(icon).size(16))
                        .push(widget::text::body(t.as_str()))
                        .into(),
                );
            }
        }
        let sp = cosmic::theme::spacing();
        widget::scrollable(
            widget::container(settings::view_column(sections))
                .padding(sp.space_m)
                .width(Length::Fill),
        )
        .into()
    }
}

impl App {
    /// The setup checklist, shown while setup is running or incomplete.
    fn setup_rows<'a>(
        &self,
        mut sec: settings::Section<'a, Message>,
    ) -> settings::Section<'a, Message> {
        for (title, desc, done) in rclone_ui_core::offline_setup::setup_steps(self.st.checks) {
            let (icon, status) = if done {
                ("emblem-ok-symbolic", fl!("settings-step-done"))
            } else {
                (
                    "dialog-information-symbolic",
                    fl!("settings-step-todo"),
                )
            };
            sec = sec.add(
                settings::item::builder(title)
                    .description(desc)
                    .icon(widget::icon::from_name(icon).size(16))
                    .control(widget::text::caption(status)),
            );
        }
        // Setup starts from the switch; this button only picks up after something undid part
        // of it (say the drop-in was deleted). Both go through the consent dialog.
        let hint = self.st.setup_hint();
        sec.add(if self.st.busy {
            settings::item::builder(fl!("settings-setting-up"))
                .description(hint)
                .control(widget::Space::new())
        } else {
            settings::item::builder(fl!("settings-setup-incomplete"))
                .description(hint)
                .control(
                    widget::button::suggested(fl!("settings-setup-button"))
                        .on_press(Message::SetUp),
                )
        })
    }

    fn offline_section(&self, now: i64) -> Element<'_, Message> {
        let mut sec = settings::section()
            .title(fl!("offline-heading"))
            .add(
                settings::item::builder(fl!("settings-keep-offline-switch"))
                    .description(self.st.description())
                    .toggler_maybe(
                        self.st.toggle_checked(),
                        self.st.toggle_allowed().then_some(Message::ToggleOffline),
                    ),
            );
        if !self.st.cfg.offline_enabled {
            return sec.into();
        }
        if self.st.show_setup() {
            return self.setup_rows(sec).into();
        }
        if self.st.cfg.pins.is_empty() {
            sec = sec.add(
                settings::item::builder(fl!("settings-no-pins"))
                    .description(fl!("cosmic-settings-no-pins-hint"))
                    .control(widget::Space::new()),
            );
        }
        for row in self.st.pin_rows(now) {
            let icon = widget::icon::from_name(row.icon.clone()).size(16);
            if row.removing {
                // Removed but still being cleaned up: no buttons.
                let desc = if row.error.is_empty() {
                    row.label
                } else {
                    format!("{}\n{}", row.label, row.error)
                };
                sec = sec.add(
                    settings::item::builder(row.rel)
                        .description(desc)
                        .icon(icon)
                        .control(widget::Space::new()),
                );
                continue;
            }
            let mut buttons = widget::row::with_capacity(3).spacing(8);
            if row.can_repair {
                buttons = buttons.push(
                    widget::button::standard(fl!("common-repair"))
                        .on_press(Message::Repair(row.rel.clone())),
                );
            }
            buttons = buttons
                .push(
                    widget::button::standard(fl!("common-open"))
                        .on_press(Message::Open(row.rel.clone())),
                )
                .push(
                    widget::button::destructive(fl!("common-remove"))
                        .on_press(Message::Unpin(row.rel.clone())),
                );
            sec = sec.add(
                settings::item::builder(row.rel)
                    .description(row.label)
                    .icon(icon)
                    .control(buttons),
            );
        }
        sec.into()
    }

    fn notices_section(notices: Vec<(String, String)>) -> Element<'static, Message> {
        let mut sec = settings::section().title(fl!("settings-heading-activity"));
        for (text, when) in notices {
            sec = sec.add(
                settings::item::builder(text)
                    .description(when)
                    .control(widget::Space::new()),
            );
        }
        sec.into()
    }
}

/// Open the window and block until it closes.
pub fn run(cfg: Config) -> cosmic::iced::Result {
    rclone_ui_i18n::init();
    let settings = Settings::default().size(Size::new(600.0, 680.0));
    cosmic::app::run::<App>(settings, cfg)
}
