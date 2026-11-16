// SPDX-License-Identifier: GPL-3.0-only
//! The "rclone…" window opened from COSMIC Files' right-click menu (`--menu PATH...`).
//! COSMIC Files can't nest custom actions in a submenu, so it has one entry that launches this
//! window. It shows the item's sync status, who it is shared with, and the actions that apply.
//!
//! Data arrives in stages (what the item is → rclone's local view → Google Drive). Rather than
//! have every field show its own loading state and the layout shift as each lands, the window
//! shows one animated loading screen (a cloud inside a spinning pair of chasing arrows) until
//! everything has arrived, then swaps to the finished layout in one step. That layout is fixed
//! anyway: a non-resizable window, fixed-height sections and fixed column widths.

use crate::config::{Config, ConfigExt};
use crate::drive::{self, DriveInfo, Kind};
use crate::keeper;
use crate::mime_icon;
use crate::share::{self, Target};
use crate::status::{self, Local};
use cosmic::app::Settings;
use cosmic::iced::advanced::text::{Ellipsize, EllipsizeHeightLimit};
use cosmic::iced::{
    time, Alignment, Length, Limits, Padding, Radians, Rotation, Size, Subscription,
};
use cosmic::prelude::*;
use cosmic::widget;
use rclone_ui_i18n::fl;
use rclone_ui_view::details::{
    self, offline_snapshot, parent_dir, permission_rows, DetailsState, OfflineButton,
    OfflineSnapshot, OpenButton, PermissionRow,
};
use rclone_ui_view::{remaining_loading, REFRESH};
use std::time::{Duration, Instant};

const WIDTH: f32 = 480.0;
const HEIGHT: f32 = 680.0;
const IDENTITY_H: f32 = 40.0;
const STATUS_H: f32 = 152.0;
const DETAIL_ROW_H: f32 = 24.0;
const DETAIL_LABEL_W: f32 = 132.0;
const PERM_ROW_H: f32 = 40.0;
const OFFLINE_ROW_H: f32 = 40.0;
const ROLE_W: f32 = 96.0;
const TYPE_W: f32 = 136.0;
/// Keeps column headers aligned with rows that sit beside the scrollbar.
const SCROLLBAR_GUTTER: f32 = 12.0;
/// One full turn of the spinner.
const SPIN_PERIOD: Duration = Duration::from_millis(1800);
const SPINNER_SIZE: u16 = 128;
const FRAME: Duration = Duration::from_millis(16);
const SKELETON_ROWS: usize = 3;

#[derive(Debug, Clone)]
enum Message {
    Resolved(Result<Target, String>),
    Local(Result<Local, String>),
    Drive(Result<DriveInfo, String>),
    ManageSharing,
    /// Expand "Open" into "Folder" / "File".
    ToggleOpen,
    /// Open this path with the default app (a folder opens in the file manager).
    OpenPath(String),
    /// Animation frame while loading.
    Tick(Instant),
    /// Turn "Keep offline" on or off for this folder.
    SetOffline(bool),
    OfflineSet(Result<String, String>),
    /// Time to re-read the keeper's progress.
    OfflineTick,
    OfflineRefreshed(Box<OfflineSnapshot>),
}

struct Menu {
    core: cosmic::Core,
    /// What has arrived, and every decision about what to show.
    state: DetailsState,
    /// "Open" has been clicked and shows its "Folder" / "File" choices.
    open_expanded: bool,
    /// A refresh of `local` is in flight (so ticks don't stack them up).
    local_busy: bool,
    started: Instant,
    /// The time of the latest animation frame.
    now: Instant,
}

struct Flags {
    cfg: Config,
    paths: Vec<String>,
}

type Task = cosmic::app::Task<Message>;

fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
    msg: impl Fn(T) -> Message + Send + 'static,
    fallback: impl Fn(String) -> T + Send + 'static,
) -> Task {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(f)
                .await
                .unwrap_or_else(|e| fallback(e.to_string()))
        },
        move |r| cosmic::Action::App(msg(r)),
    )
}

impl cosmic::Application for Menu {
    type Executor = cosmic::executor::Default;
    type Flags = Flags;
    type Message = Message;
    const APP_ID: &'static str = crate::APP_ID;

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(mut core: cosmic::Core, flags: Flags) -> (Self, Task) {
        core.window.show_maximize = false;
        core.window.show_minimize = false;
        core.set_header_title(fl!("details-window-title"));
        let menu = Menu {
            core,
            state: DetailsState::new(flags.cfg.clone(), &flags.paths),
            open_expanded: false,
            local_busy: false,
            started: Instant::now(),
            now: Instant::now(),
        };
        let (cfg, paths) = (flags.cfg, flags.paths);
        let task = blocking(move || share::resolve(&cfg, &paths), Message::Resolved, Err);
        (menu, task)
    }

    fn update(&mut self, message: Message) -> Task {
        match message {
            Message::Resolved(r) => {
                let task = match &r {
                    Ok(t) => {
                        let (cfg, cfg2) = (self.state.cfg.clone(), self.state.cfg.clone());
                        let (t1, t2) = (t.clone(), t.clone());
                        self.local_busy = true;
                        Task::batch([
                            blocking(move || status::local(&cfg, &t1), Message::Local, Err),
                            blocking(move || drive::fetch(&cfg2, &t2), Message::Drive, Err),
                        ])
                    }
                    Err(_) => Task::none(),
                };
                self.state.apply_target(r);
                return task;
            }
            Message::Local(r) => {
                self.local_busy = false;
                self.state.apply_local(r);
            }
            Message::Drive(r) => self.state.apply_drive(r),
            Message::Tick(now) => self.now = now,
            Message::ToggleOpen => self.open_expanded = !self.open_expanded,
            Message::SetOffline(on) => {
                let path = self.state.path.clone();
                return blocking(
                    move || keeper::set_pinned(&path, on),
                    Message::OfflineSet,
                    Err,
                );
            }
            Message::OfflineSet(r) => match r {
                Ok(_) => {
                    self.state.offline_error = None;
                    self.state.cfg = Config::load();
                    return self.update(Message::OfflineTick);
                }
                Err(e) => self.state.offline_error = Some(e),
            },
            Message::OfflineTick => {
                let mut tasks = vec![];
                if self.state.cfg.offline_enabled {
                    let path = self.state.path.clone();
                    tasks.push(blocking(
                        move || Box::new(offline_snapshot(&path)),
                        Message::OfflineRefreshed,
                        |_| Box::new(OfflineSnapshot::default()),
                    ));
                }
                // Transfers and queues change while the window is open; keep the counts live.
                if !self.local_busy {
                    if let Some(t) = self.state.target.ok().cloned() {
                        self.local_busy = true;
                        let cfg = self.state.cfg.clone();
                        tasks.push(blocking(
                            move || status::local(&cfg, &t),
                            Message::Local,
                            Err,
                        ));
                    }
                }
                return Task::batch(tasks);
            }
            Message::OfflineRefreshed(snap) => self.state.apply_snapshot(*snap),
            Message::OpenPath(p) => {
                share::open_url(&p);
                std::process::exit(0);
            }
            Message::ManageSharing => {
                if let Some(t) = self.state.target.ok() {
                    share::open_url(&share::sharing_url(t));
                }
                // The browser is launched detached, so there is nothing left to wait for.
                std::process::exit(0);
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        if self.loading() {
            time::every(FRAME).map(Message::Tick)
        } else {
            // Follow transfers, the queue and the offline keeper while the window is open.
            time::every(REFRESH).map(|_| Message::OfflineTick)
        }
    }

    fn view(&self) -> Element<'_, Message> {
        if self.loading() {
            return self.loading_view();
        }
        let sp = cosmic::theme::spacing();
        let body = widget::column::with_capacity(8)
            .spacing(sp.space_xs)
            .push(self.identity())
            .push(widget::divider::horizontal::default())
            .push(self.status_section())
            .push(widget::divider::horizontal::default())
            .push(self.sharing_header())
            .push(self.sharing_list())
            .push(widget::divider::horizontal::default());
        let body = match self.offline_row() {
            Some(row) => body.push(row).push(widget::divider::horizontal::default()),
            None => body,
        };
        let body = body.push(self.actions());
        widget::container(body)
            .padding(sp.space_s)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

impl Menu {
    /// Still waiting on data, or the loading screen hasn't been up long enough yet.
    fn loading(&self) -> bool {
        !self.state.all_landed()
            || !remaining_loading(self.now.duration_since(self.started)).is_zero()
    }

    /// A cloud with two arrows chasing each other around it, centred in the window.
    fn loading_view(&self) -> Element<'_, Message> {
        let layer = |svg: &'static [u8]| {
            let mut handle = widget::icon::from_svg_bytes(svg);
            handle.symbolic = true;
            widget::icon(handle).size(SPINNER_SIZE)
        };
        let turns = self.now.duration_since(self.started).as_secs_f32() / SPIN_PERIOD.as_secs_f32();
        let angle = turns.fract() * std::f32::consts::TAU;
        let spinner = cosmic::iced::widget::stack![
            layer(include_bytes!("../../resources/loading/ring.svg"))
                .rotation(Rotation::Floating(Radians(angle))),
            layer(include_bytes!("../../resources/loading/cloud.svg")),
        ]
        .width(Length::Fixed(f32::from(SPINNER_SIZE)))
        .height(Length::Fixed(f32::from(SPINNER_SIZE)));
        let col = widget::column::with_capacity(2)
            .spacing(cosmic::theme::spacing().space_s)
            .align_x(Alignment::Center)
            .push(spinner)
            .push(widget::text::body(fl!("details-checking-sync-status")));
        widget::container(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .into()
    }

    // --- 1. identity: known immediately, never changes ---

    fn identity(&self) -> Element<'_, Message> {
        let s = &self.state;
        let icon: Element<'_, Message> = if s.is_dir {
            widget::icon::from_name("folder-symbolic").size(32).into()
        } else {
            widget::icon(mime_icon::file_icon(&s.name, 32))
                .size(32)
                .into()
        };
        let text = widget::column::with_capacity(2)
            .width(Length::Fill)
            .push(widget::text::heading(&s.name).ellipsize(Ellipsize::Middle(lines())))
            .push(widget::text::caption(&s.path).ellipsize(Ellipsize::Start(lines())));
        let row = widget::row::with_capacity(2)
            .spacing(cosmic::theme::spacing().space_xs)
            .align_y(Alignment::Center)
            .push(icon)
            .push(text);
        widget::container(row)
            .height(Length::Fixed(IDENTITY_H))
            .align_y(Alignment::Center)
            .into()
    }

    // --- 2. sync status: always a title, a status line and a fixed set of detail rows ---

    fn status_section(&self) -> Element<'_, Message> {
        let (icon, label, detail) = self.state.status_line();
        let line = widget::row::with_capacity(3)
            .spacing(cosmic::theme::spacing().space_xxs)
            .align_y(Alignment::Center)
            .push(widget::icon::from_name(icon).size(16))
            .push(widget::text::body(label).ellipsize(Ellipsize::End(lines())))
            .push(
                widget::container(widget::text::caption(detail).ellipsize(Ellipsize::End(lines())))
                    .width(Length::Fill)
                    .align_x(Alignment::End),
            );
        let mut col = widget::column::with_capacity(6)
            .push(
                widget::container(widget::text::heading(fl!("details-heading-sync-status")))
                    .height(Length::Fixed(20.0))
                    .align_y(Alignment::Center),
            )
            .push(
                widget::container(line)
                    .height(Length::Fixed(32.0))
                    .align_y(Alignment::Center),
            );
        for row in self.state.detail_rows() {
            let (label, value) = (row.label, row.value);
            col = col.push(
                widget::container(
                    widget::row::with_capacity(2)
                        .align_y(Alignment::Center)
                        .push(
                            widget::container(widget::text::caption(label))
                                .width(Length::Fixed(DETAIL_LABEL_W)),
                        )
                        .push(
                            widget::container(
                                widget::text::body(value).ellipsize(Ellipsize::End(lines())),
                            )
                            .width(Length::Fill),
                        ),
                )
                .height(Length::Fixed(DETAIL_ROW_H))
                .align_y(Alignment::Center),
            );
        }
        widget::container(col)
            .height(Length::Fixed(if self.state.is_dir {
                STATUS_H - DETAIL_ROW_H
            } else {
                STATUS_H
            }))
            .width(Length::Fill)
            .into()
    }

    // --- 3. sharing: header and column titles are fixed; the list scrolls in a fixed viewport ---

    fn sharing_header(&self) -> Element<'_, Message> {
        let title = widget::container(
            widget::row::with_capacity(2)
                .align_y(Alignment::Center)
                .push(widget::text::heading(fl!("details-heading-sharing")).width(Length::Fill))
                .push(widget::text::caption(self.state.sharing_summary())),
        )
        .height(Length::Fixed(24.0))
        .align_y(Alignment::Center);
        widget::column::with_capacity(3)
            .spacing(cosmic::theme::spacing().space_xxs)
            .push(title)
            .push(
                widget::container(perm_columns(
                    widget::text::caption_heading(fl!("details-column-name")).into(),
                    widget::text::caption_heading(fl!("details-column-role")).into(),
                    widget::text::caption_heading(fl!("details-column-type")).into(),
                ))
                .height(Length::Fixed(20.0)),
            )
            .push(widget::divider::horizontal::light())
            .into()
    }

    fn sharing_list(&self) -> Element<'_, Message> {
        let mut col = widget::column::with_capacity(8).width(Length::Fill);
        if self.state.drive.waiting() {
            for _ in 0..SKELETON_ROWS {
                let dots = || widget::text::caption(details::PLACEHOLDER).into();
                col = col.push(perm_row(dots(), dots(), dots()));
            }
        } else if let Some((icon, message)) = self.state.sharing_note() {
            col = col.push(note_row(icon, message));
        } else {
            for (i, p) in permission_rows(self.state.permissions())
                .into_iter()
                .enumerate()
            {
                if i > 0 {
                    col = col.push(widget::divider::horizontal::light());
                }
                col = col.push(permission_row(p));
            }
        }
        widget::scrollable(col).height(Length::Fill).into()
    }

    // --- offline: only when offline folders are on ---

    /// The "Keep offline" switch for a folder, or a note for things inside an offline folder.
    fn offline_row(&self) -> Option<Element<'_, Message>> {
        let row = self.state.offline_row()?;
        let text = widget::column::with_capacity(2)
            .width(Length::Fill)
            .push(widget::text::body(row.title))
            .push(widget::text::caption(row.caption).ellipsize(Ellipsize::End(lines())));
        let mut line = widget::row::with_capacity(3)
            .spacing(cosmic::theme::spacing().space_xs)
            .align_y(Alignment::Center)
            .push(widget::icon::from_name("folder-download-symbolic").size(16))
            .push(text);
        match row.button {
            OfflineButton::Hidden => {}
            OfflineButton::Keep => {
                line = line.push(widget::toggler(false).on_toggle(Message::SetOffline));
            }
            OfflineButton::Stop => {
                line = line.push(widget::toggler(true).on_toggle(Message::SetOffline));
            }
        }
        Some(
            widget::container(line)
                .height(Length::Fixed(OFFLINE_ROW_H))
                .align_y(Alignment::Center)
                .into(),
        )
    }

    // --- 4. actions: pinned last, present from the start ---

    fn actions(&self) -> Element<'_, Message> {
        let buttons = self.state.buttons();
        let path = &self.state.path;
        // "Open" expands in place into "Folder" / "File", so it keeps the same slot and the row
        // never changes size. A folder has nothing to choose between, so it opens directly.
        let open: Element<'_, Message> = match buttons.open {
            OpenButton::Direct => widget::button::standard(fl!("common-open"))
                .on_press(Message::OpenPath(path.clone()))
                .width(Length::Fill)
                .into(),
            OpenButton::Menu if self.open_expanded => widget::row::with_capacity(2)
                .spacing(cosmic::theme::spacing().space_xxs)
                .push(
                    widget::button::standard(fl!("details-open-folder"))
                        .on_press(Message::OpenPath(parent_dir(path)))
                        .width(Length::Fill),
                )
                .push(
                    widget::button::standard(fl!("details-open-file"))
                        .on_press(Message::OpenPath(path.clone()))
                        .width(Length::Fill),
                )
                .into(),
            OpenButton::Menu => widget::button::standard(fl!("common-open"))
                .on_press(Message::ToggleOpen)
                .width(Length::Fill)
                .into(),
        };
        widget::container(
            widget::row::with_capacity(2)
                .spacing(cosmic::theme::spacing().space_xxs)
                .push(widget::container(open).width(Length::FillPortion(2)))
                .push(
                    widget::button::suggested(buttons.manage_label)
                        .on_press_maybe(buttons.manage_enabled.then_some(Message::ManageSharing))
                        .width(Length::FillPortion(3)),
                ),
        )
        .height(Length::Fixed(32.0))
        .width(Length::Fill)
        .into()
    }
}

fn lines() -> EllipsizeHeightLimit {
    EllipsizeHeightLimit::Lines(1)
}

/// Name / Role / Type with the shared column widths.
fn perm_columns<'a>(
    name: Element<'a, Message>,
    role: Element<'a, Message>,
    kind: Element<'a, Message>,
) -> Element<'a, Message> {
    widget::row::with_capacity(3)
        .spacing(cosmic::theme::spacing().space_xxs)
        .align_y(Alignment::Center)
        .padding(Padding {
            right: SCROLLBAR_GUTTER,
            ..Padding::ZERO
        })
        .push(widget::container(name).width(Length::Fill))
        .push(widget::container(role).width(Length::Fixed(ROLE_W)))
        .push(widget::container(kind).width(Length::Fixed(TYPE_W)))
        .into()
}

fn perm_row<'a>(
    name: Element<'a, Message>,
    role: Element<'a, Message>,
    kind: Element<'a, Message>,
) -> Element<'a, Message> {
    widget::container(perm_columns(name, role, kind))
        .height(Length::Fixed(PERM_ROW_H))
        .align_y(Alignment::Center)
        .into()
}

fn permission_row(p: PermissionRow) -> Element<'static, Message> {
    let mut name = widget::column::with_capacity(2)
        .push(widget::text::body(p.name).ellipsize(Ellipsize::End(lines())));
    if !p.address.is_empty() {
        name = name.push(widget::text::caption(p.address).ellipsize(Ellipsize::Middle(lines())));
    }
    let name = widget::tooltip(
        name,
        widget::text::body(p.full),
        widget::tooltip::Position::Bottom,
    );
    // The theme names are COSMIC's; the grantee kinds come from the shared view.
    let icon = match p.kind {
        Kind::User => "avatar-default-symbolic",
        Kind::Group => "system-users-symbolic",
        Kind::Domain => "network-workgroup-symbolic",
        Kind::Anyone => "insert-link-symbolic",
    };
    let kind = widget::row::with_capacity(2)
        .spacing(cosmic::theme::spacing().space_xxs)
        .align_y(Alignment::Center)
        .push(widget::icon::from_name(icon).size(16))
        .push(widget::text::caption(p.kind_label).ellipsize(Ellipsize::End(lines())));
    perm_row(name.into(), widget::text::body(p.role).into(), kind.into())
}

/// A single full-width row for empty and error states, same height as a data row.
fn note_row(icon: &'static str, text: String) -> Element<'static, Message> {
    // The tooltip carries the whole message when an error is too long for the row.
    let label = widget::tooltip(
        widget::text::body(text.clone()).ellipsize(Ellipsize::End(lines())),
        widget::text::body(text),
        widget::tooltip::Position::Bottom,
    );
    widget::container(
        widget::row::with_capacity(2)
            .spacing(cosmic::theme::spacing().space_xxs)
            .align_y(Alignment::Center)
            .push(widget::icon::from_name(icon).size(16))
            .push(label),
    )
    .height(Length::Fixed(PERM_ROW_H))
    .width(Length::Fill)
    .align_y(Alignment::Center)
    .into()
}

/// Open the window and block until it closes.
pub fn run(cfg: Config, paths: Vec<String>) -> cosmic::iced::Result {
    let settings = Settings::default()
        .size(Size::new(WIDTH, HEIGHT))
        // `resizable(None)` alone doesn't stop resizing; pinning min = max does.
        .size_limits(
            Limits::NONE
                .min_width(WIDTH)
                .max_width(WIDTH)
                .min_height(HEIGHT)
                .max_height(HEIGHT),
        )
        .resizable(None);
    cosmic::app::run::<Menu>(settings, Flags { cfg, paths })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::Provider;
    use cosmic::Application as _;
    use rclone_ui_view::{Load, MIN_LOADING};

    fn menu() -> Menu {
        let now = Instant::now();
        Menu {
            core: cosmic::Core::default(),
            state: DetailsState::new(Config::default(), &["/m/d/a.txt".to_string()]),
            open_expanded: false,
            local_busy: false,
            started: now,
            now,
        }
    }

    fn target() -> Target {
        Target {
            provider: Provider::GoogleDrive,
            id: "id".into(),
            is_dir: false,
            fs: "gdrive:".into(),
            rel: "d/a.txt".into(),
        }
    }

    #[test]
    fn the_spinner_stays_for_the_minimum_time_even_when_everything_has_landed() {
        let mut m = menu();
        m.state.apply_target(Err("no".into()));
        assert!(m.state.all_landed());
        assert!(m.loading());
        m.now = m.started + MIN_LOADING;
        assert!(!m.loading());
    }

    #[test]
    fn ticks_advance_the_animation_clock() {
        let mut m = menu();
        let later = m.started + Duration::from_millis(300);
        let _ = m.update(Message::Tick(later));
        assert_eq!(m.now, later);
    }

    #[test]
    fn resolving_starts_the_other_lookups_without_finishing_them() {
        let mut m = menu();
        let _ = m.update(Message::Resolved(Ok(target())));
        assert!(m.state.target.ok().is_some());
        assert!(m.state.local.waiting() && m.state.drive.waiting());
        assert!(m.local_busy);
    }

    #[test]
    fn results_land_in_the_right_slot() {
        let mut m = menu();
        m.local_busy = true;
        let _ = m.update(Message::Local(Err("rc down".into())));
        let _ = m.update(Message::Drive(Err("boom".into())));
        assert!(!m.local_busy);
        assert_eq!(m.state.local.err(), Some("rc down"));
        assert_eq!(m.state.drive.err(), Some("boom"));
        assert!(matches!(m.state.target, Load::Pending));
    }

    #[test]
    fn open_expands_and_collapses() {
        let mut m = menu();
        let _ = m.update(Message::ToggleOpen);
        assert!(m.open_expanded);
        let _ = m.update(Message::ToggleOpen);
        assert!(!m.open_expanded);
    }

    #[test]
    fn every_state_builds_a_view_without_panicking() {
        for is_dir in [false, true] {
            let mut m = menu();
            m.state.is_dir = is_dir;
            let _ = m.view();
            m.now = m.started + MIN_LOADING;
            m.state.apply_target(Ok(target()));
            m.state.apply_local(Ok(Local::File {
                transferring: false,
                cached: None,
            }));
            m.state.apply_drive(Ok(DriveInfo::default()));
            for open in [false, true] {
                m.open_expanded = open;
                let _ = m.view();
            }
            m.state.apply_drive(Err("denied".into()));
            let _ = m.view();
        }
    }
}
