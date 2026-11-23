// SPDX-License-Identifier: GPL-3.0-only
//! The settings window (`--settings`, or "Settings…" in the panel popup).
//!
//! Its main job is offline folders. What the window says and the consent -> setup -> rollback
//! flow live in `rclone_ui_view::settings` (shared with the Plasma window); the Backups and
//! Restore sections' rows and labels live in `rclone_ui_view::backup`. This file is the iced
//! widgets, messages and worker tasks around them.

use crate::config::{Config, ConfigExt, ConfigWatch, WATCH_EVERY};
use crate::offline_setup::{set_up, tear_down, turn_on, Checks};
use crate::pin::{self, OfflineState};
use crate::{keeper, share};
use cosmic::app::Settings;
use cosmic::dialog::file_chooser;
use cosmic::iced::{time, Alignment, Length, Size, Subscription};
use cosmic::prelude::*;
use cosmic::widget::{self, settings};
use rclone_ui_core::backup::{self as core_backup, BackupStatus, RemoteBackup};
use rclone_ui_i18n::fl;
use rclone_ui_view::backup::{self, RestoreRow};
use rclone_ui_view::settings::{ConsentAction, SettingsState, ToggleAction};
use rclone_ui_view::REFRESH;

#[derive(Debug, Clone)]
enum Message {
    Refresh,
    Refreshed(Checks, OfflineState, BackupStatus),
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
    /// "Back up a folder…": open the folder picker.
    BackupPick,
    /// The folder picked to back up (`None`: the picker was cancelled).
    BackupPicked(Result<Option<String>, String>),
    BackupRemove(String),
    /// `backup::add` or `remove` finished; the error is a failure code.
    BackupChanged(Result<(), String>),
    /// "Find backups in Drive".
    RestoreLoad,
    /// The backups in Drive, or a failure code.
    RestoreLoaded(Result<Vec<RemoteBackup>, String>),
    RestoreLink(bool),
    /// "Restore to…" on the backup at this remote destination: open the folder picker.
    RestorePick(String),
    /// Where to restore that backup (`None`: the picker was cancelled).
    RestorePicked(String, Result<Option<String>, String>),
    /// The restore process started (or the message why it couldn't), for this local folder.
    RestoreStarted(String, Result<(), String>),
}

/// The Restore section's list of backups in Drive.
enum RestoreList {
    /// Not asked for yet: listing runs rclone over the network, so it waits for the button.
    NotLoaded,
    Loading,
    Loaded(Vec<RestoreRow>),
}

struct App {
    core: cosmic::Core,
    st: SettingsState,
    config_watch: ConfigWatch,
    /// The backup keeper's last report, read on every refresh.
    backups: BackupStatus,
    /// A backup is being added or removed: its buttons wait.
    backup_busy: bool,
    /// The Backups section's description; it names this machine, which doesn't change.
    backup_description: String,
    restore: RestoreList,
    /// "Keep backing up the restored folder".
    restore_link: bool,
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

/// Ask for a folder through the desktop's file chooser portal. `Ok(None)` when cancelled.
fn pick_folder(
    title: String,
    msg: impl Fn(Result<Option<String>, String>) -> Message + Send + 'static,
) -> Task {
    Task::perform(
        async move {
            match file_chooser::open::Dialog::new()
                .title(title)
                .open_folder()
                .await
            {
                // A location that isn't a local path is passed on as its URL, so the backup
                // code rejects it with its own "not a local folder" error.
                Ok(r) => Ok(Some(r.url().to_file_path().map_or_else(
                    |()| r.url().to_string(),
                    |p| p.to_string_lossy().into_owned(),
                ))),
                Err(file_chooser::Error::Cancelled) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        },
        move |r| cosmic::Action::App(msg(r)),
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
            backups: BackupStatus::default(),
            backup_busy: false,
            backup_description: backup::description(),
            restore: RestoreList::NotLoaded,
            restore_link: false,
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
                    move || {
                        (
                            Checks::probe(&cfg),
                            OfflineState::load(),
                            // An unreadable report shows the folders as not backed up yet.
                            core_backup::status().unwrap_or_default(),
                        )
                    },
                    |(c, s, b)| Message::Refreshed(c, s, b),
                );
            }
            Message::Refreshed(c, s, b) => {
                self.st.checks = c;
                self.st.state = s;
                self.backups = b;
            }
            Message::ConfigTick => {
                if let Some(c) = self.config_watch.poll() {
                    self.st.apply_config(c);
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
                // Also where a backup task lands if its worker panicked: let its buttons go.
                self.backup_busy = false;
                if matches!(self.restore, RestoreList::Loading) {
                    self.restore = RestoreList::NotLoaded;
                }
                // A failed turn-on switches itself back off; show that now, not on the next
                // config notification.
                self.st.finish(r, Config::load());
                return self.update(Message::Refresh);
            }
            Message::Unpin(rel) => match Config::save(|c| c.pins.retain(|p| *p != rel)) {
                Ok(c) => self.st.apply_config(c),
                Err(e) => self.st.error = Some(e),
            },
            Message::Repair(rel) => {
                if let Err(e) = keeper::request_repair(&rel) {
                    self.st.error = Some(e.to_string());
                }
            }
            Message::Open(rel) => {
                share::open_url(&pin::mount_point(&self.st.cfg, &rel).to_string_lossy());
            }
            Message::ToggleContextMenu(on) => match Config::save(|c| c.context_menu = on) {
                Ok(c) => {
                    self.st.apply_config(c);
                    if on {
                        crate::context_menu::install();
                    } else {
                        crate::context_menu::remove();
                    }
                }
                Err(e) => self.st.error = Some(e),
            },
            m @ (Message::BackupPick
            | Message::BackupPicked(_)
            | Message::BackupRemove(_)
            | Message::BackupChanged(_)
            | Message::RestoreLoad
            | Message::RestoreLoaded(_)
            | Message::RestoreLink(_)
            | Message::RestorePick(_)
            | Message::RestorePicked(..)
            | Message::RestoreStarted(..)) => return self.update_backup(m),
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
        sections.push(self.backup_section(now));
        sections.push(self.restore_section());
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
    /// The Backups and Restore sections' messages.
    fn update_backup(&mut self, message: Message) -> Task {
        match message {
            Message::BackupPick => {
                return pick_folder(fl!("settings-backup-choose"), Message::BackupPicked);
            }
            Message::BackupPicked(Ok(Some(source))) => {
                self.backup_busy = true;
                return blocking(
                    move || core_backup::add(&source).map(drop),
                    Message::BackupChanged,
                );
            }
            Message::BackupPicked(Err(e)) | Message::RestorePicked(_, Err(e)) => {
                self.st.error = Some(e);
            }
            Message::BackupRemove(source) => {
                self.backup_busy = true;
                return blocking(
                    move || core_backup::remove(&source).map(drop),
                    Message::BackupChanged,
                );
            }
            Message::BackupChanged(r) => {
                self.backup_busy = false;
                match r {
                    Ok(()) => {
                        self.st.error = None;
                        self.st.apply_config(Config::load());
                        return cosmic::Application::update(self, Message::Refresh);
                    }
                    Err(code) => self.st.error = Some(backup::error(&code)),
                }
            }
            Message::RestoreLoad => {
                self.restore = RestoreList::Loading;
                return blocking(core_backup::list_remote, Message::RestoreLoaded);
            }
            Message::RestoreLoaded(Ok(list)) => {
                self.restore = RestoreList::Loaded(backup::restore_rows(&list));
            }
            Message::RestoreLoaded(Err(code)) => {
                self.restore = RestoreList::NotLoaded;
                self.st.error = Some(backup::error(&code));
            }
            Message::RestoreLink(on) => self.restore_link = on,
            Message::RestorePick(destination) => {
                return pick_folder(fl!("settings-restore-choose"), move |r| {
                    Message::RestorePicked(destination.clone(), r)
                });
            }
            Message::RestorePicked(destination, Ok(Some(target))) => {
                let link = self.restore_link;
                return blocking(
                    move || {
                        let r = core_backup::spawn_restore(&destination, &target, link);
                        (target, r)
                    },
                    |(target, r)| Message::RestoreStarted(target, r),
                );
            }
            Message::RestoreStarted(target, Ok(())) => {
                self.st.note = Some(backup::restore_started(&target));
                self.st.error = None;
            }
            Message::RestoreStarted(_, Err(e)) => self.st.error = Some(e),
            // A cancelled picker, or a message `update` handles.
            _ => {}
        }
        Task::none()
    }

    /// The folders this machine backs up, each with "Stop backing up", and "Back up a folder…".
    fn backup_section(&self, now: i64) -> Element<'_, Message> {
        let mut sec = settings::section()
            .title(fl!("settings-heading-backups"))
            .add(widget::text::body(self.backup_description.as_str()));
        let rows = backup::rows(&self.st.cfg, &self.backups, now);
        if rows.is_empty() {
            sec = sec.add(widget::text::body(fl!("settings-backup-none")));
        }
        for row in rows {
            let icon = if row.error {
                "dialog-error-symbolic"
            } else {
                "folder-symbolic"
            };
            let stop = widget::button::destructive(fl!("backup-menu-remove")).on_press_maybe(
                (!self.backup_busy).then(|| Message::BackupRemove(row.source.clone())),
            );
            sec = sec.add(
                settings::item::builder(row.source)
                    .description(format!("{}\n{}", row.destination, row.status))
                    .icon(widget::icon::from_name(icon).size(16))
                    .control(stop),
            );
        }
        sec.add(settings::item_row(vec![
            widget::space::horizontal().into(),
            widget::button::standard(fl!("settings-backup-add"))
                .on_press_maybe((!self.backup_busy).then_some(Message::BackupPick))
                .into(),
        ]))
        .into()
    }

    /// "Find backups in Drive", then every machine's backups with "Restore to…".
    fn restore_section(&self) -> Element<'_, Message> {
        let loading = matches!(self.restore, RestoreList::Loading);
        let mut sec = settings::section()
            .title(fl!("settings-restore-heading"))
            .add(settings::item_row(vec![
                widget::space::horizontal().into(),
                widget::button::standard(fl!("settings-restore-load"))
                    .on_press_maybe((!loading).then_some(Message::RestoreLoad))
                    .into(),
            ]));
        match &self.restore {
            RestoreList::NotLoaded => {}
            RestoreList::Loading => {
                sec = sec.add(widget::text::body(fl!("settings-restore-loading")));
            }
            RestoreList::Loaded(rows) if rows.is_empty() => {
                sec = sec.add(widget::text::body(fl!("settings-restore-none")));
            }
            RestoreList::Loaded(rows) => {
                // Applies to whichever backup is restored next.
                sec = sec.add(
                    settings::item::builder(fl!("settings-restore-link"))
                        .checkbox(self.restore_link, Message::RestoreLink),
                );
                for row in rows {
                    sec = sec.add(
                        settings::item::builder(row.label.as_str()).control(
                            widget::button::standard(fl!("settings-restore-button"))
                                .on_press(Message::RestorePick(row.destination.clone())),
                        ),
                    );
                }
            }
        }
        sec.into()
    }

    /// The setup checklist, shown while setup is running or incomplete.
    fn setup_rows<'a>(
        &self,
        mut sec: settings::Section<'a, Message>,
    ) -> settings::Section<'a, Message> {
        for (title, desc, done) in rclone_ui_core::offline_setup::setup_steps(self.st.checks) {
            let (icon, status) = if done {
                ("emblem-ok-symbolic", fl!("settings-step-done"))
            } else {
                ("dialog-information-symbolic", fl!("settings-step-todo"))
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
        let mut sec = settings::section().title(fl!("offline-heading")).add(
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
