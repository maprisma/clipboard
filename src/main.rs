mod window_size;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime};

use cosmic::app::{Core, Task};
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::iced::platform_specific::shell::wayland::commands::popup::destroy_popup;
use cosmic::iced::widget::{container as iced_container, image as iced_image, svg as iced_svg, text as iced_text};
use cosmic::iced::{Alignment, Background, Border, Color, ContentFit, Length, Rectangle, Subscription, keyboard, window};
use cosmic::widget::rectangle_tracker::{RectangleTracker, RectangleUpdate, rectangle_tracker_subscription};
use cosmic::widget::{
    Id, autosize, button, column, container, divider, flex_row, icon, image as cosmic_image, row, scrollable, space,
    svg, text, text_input, toggler,
};
use cosmic::{ApplicationExt, Element, theme};
use maprisma_clipboard::i18n::{self, tr, trf};
use maprisma_clipboard::store::{
    CategoryFilter, ClipEntry, ClipKind, ClipboardStore, ColorRgba, format_bytes, now_unix, relative_time,
};
use maprisma_clipboard::{
    APP_ID, APP_NAME, AppPrefs, CLOSE_WINDOW, HISTORY_LIMITS, TRAY_BUS, WINDOW_APP_ID, WINDOW_BUS, WindowControl,
    clipboard, corners, data_dir, load_prefs, look, shot,
};
use serde_json::Value;

static AUTOSIZE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("clipboard-autosize"));
static SEARCH_ID: LazyLock<Id> = LazyLock::new(|| Id::new("clipboard-search"));
static NOTE_ID: LazyLock<Id> = LazyLock::new(|| Id::new("clipboard-shot-note"));

const SHOT_FOLDER: &str = APP_NAME;
const POPUP_WIDTH: f32 = 460.0;
const POPUP_HEIGHT: f32 = 700.0;
const POPUP_MAX_HEIGHT: f32 = 760.0;

// Inline symbolic SVGs so every glyph renders crisply across COSMIC, KDE & Hyprland icon themes.
const GLYPH_CLIPBOARD: &[u8] = include_bytes!("../data/icons/io.github.ubruckhaus.CosmicMaprismaClipboard-symbolic.svg");
const GLYPH_PAUSED: &[u8] = include_bytes!("../data/icons/clipboard-paused-symbolic.svg");
const GLYPH_PIN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="currentColor" d="M9.8 1.8a1 1 0 0 1 1.4 0l3 3a1 1 0 0 1-.2 1.6l-2.3 1.2-1.1 3.3a1 1 0 0 1-1.6.4L6.6 8.9 2.7 12.8a.75.75 0 1 1-1.06-1.06l3.9-3.9-2.4-2.4a1 1 0 0 1 .4-1.6l3.3-1.1 1.2-2.3a1 1 0 0 1 .2-.2z"/></svg>"#;
const GLYPH_COPY: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="currentColor" d="M5 2a2 2 0 0 0-2 2v7h1.5V4a.5.5 0 0 1 .5-.5h6V2H5zm3 3a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2h4a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2H8zm0 1.5h4a.5.5 0 0 1 .5.5v6a.5.5 0 0 1-.5.5H8a.5.5 0 0 1-.5-.5V7a.5.5 0 0 1 .5-.5z"/></svg>"#;
const GLYPH_TRASH: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="currentColor" d="M6 2a1 1 0 0 0-1 1H2.5v1.5h11V3H11a1 1 0 0 0-1-1H6zM3.5 5.5l.7 8a1.5 1.5 0 0 0 1.5 1.4h4.6a1.5 1.5 0 0 0 1.5-1.4l.7-8H11l-.6 7.5a.4.4 0 0 1-.4.4H6a.4.4 0 0 1-.4-.4L5 5.5H3.5z"/></svg>"#;
const GLYPH_PAUSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="currentColor" d="M4 2.5h2.75v11H4v-11zm5.25 0H12v11H9.25v-11z"/></svg>"#;
const GLYPH_PLAY: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="currentColor" d="M4.2 2.4a.8.8 0 0 1 1.2-.68l8.2 5.6a.8.8 0 0 1 0 1.36l-8.2 5.6A.8.8 0 0 1 4.2 13.6V2.4z"/></svg>"#;
const GLYPH_LINK: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="currentColor" d="M6.8 11.2a3.2 3.2 0 0 1 0-4.53l1.7-1.7a3.2 3.2 0 0 1 4.53 4.53l-1.1 1.1-1.06-1.06 1.1-1.1a1.7 1.7 0 0 0-2.4-2.4l-1.7 1.7a1.7 1.7 0 0 0 0 2.4l.4.4-1.06 1.06-.41-.4z"/><path fill="currentColor" d="M9.2 4.8a3.2 3.2 0 0 1 0 4.53l-1.7 1.7a3.2 3.2 0 1 1-4.53-4.53l1.1-1.1 1.06 1.06-1.1 1.1a1.7 1.7 0 1 0 2.4 2.4l1.7-1.7a1.7 1.7 0 0 0 0-2.4l-.4-.4 1.06-1.06.41.4z"/></svg>"#;
const GLYPH_CODE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="currentColor" d="M5.4 4.2 1.6 8l3.8 3.8 1.06-1.06L3.72 8l2.74-2.74L5.4 4.2zm5.2 0L9.54 5.26 12.28 8l-2.74 2.74 1.06 1.06L14.4 8l-3.8-3.8zM8.9 2.5l-2.5 11 1.46.34 2.5-11L8.9 2.5z"/></svg>"#;
const GLYPH_TEXT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="currentColor" d="M2.5 3h11v1.5h-11V3zm0 3.2h11v1.5h-11V6.2zm0 3.2h8v1.5h-8V9.4zm0 3.1h5.5V14h-5.5v-1.5z"/></svg>"#;
const GLYPH_MAIL: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="currentColor" d="M2.5 3A1.5 1.5 0 0 0 1 4.5v7A1.5 1.5 0 0 0 2.5 13h11a1.5 1.5 0 0 0 1.5-1.5v-7A1.5 1.5 0 0 0 13.5 3h-11zm0 1.5h11v.4L8 8.6 2.5 4.9v-.4zm0 2.1L7.6 10a.75.75 0 0 0 .8 0l5.1-3.4v4.9h-11V6.6z"/></svg>"#;
const GLYPH_CLOSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="currentColor" d="M4.2 3.14 8 6.94l3.8-3.8 1.06 1.06L9.06 8l3.8 3.8-1.06 1.06L8 9.06l-3.8 3.8-1.06-1.06L6.94 8l-3.8-3.8L4.2 3.14z"/></svg>"#;

#[derive(Debug, Clone, Copy, Default)]
struct LaunchFlags {
    window_mode: bool,
    delete_mode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliAction {
    Help,
    Version,
    Applet,
    Window { delete_mode: bool },
}

fn decide_cli_mode(args: &[String], cosmic_panel_env: bool) -> CliAction {
    if args.iter().any(|a| a == "--version" || a == "-V") {
        return CliAction::Version;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return CliAction::Help;
    }
    let force_window = args
        .iter()
        .any(|a| a == "--window" || a == "--toggle" || a == "-d" || a == "--delete");
    let delete_mode = args.iter().any(|a| a == "-d" || a == "--delete");
    let force_applet = args.iter().any(|a| a == "--applet");

    if force_applet || (cosmic_panel_env && !force_window) {
        CliAction::Applet
    } else {
        CliAction::Window { delete_mode }
    }
}

async fn close_running_window(bus: &str) {
    if let Ok(connection) = zbus::Connection::session().await {
        let _ = connection
            .call_method(Some(bus), "/Window", Some(WINDOW_BUS), "Close", &())
            .await;
    }
}

fn main() -> cosmic::iced::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cosmic_panel_env =
        std::env::var_os("COSMIC_PANEL_SIZE").is_some() || std::env::var_os("COSMIC_PANEL_ANCHOR").is_some();

    match decide_cli_mode(&args, cosmic_panel_env) {
        CliAction::Version => {
            println!("maprisma-clipboard {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        CliAction::Help => {
            println!(
                "Maprisma Clipboard {}\n\nUsage:\n  maprisma-clipboard [OPTIONS]\n\nOptions:\n  --window, --toggle   Open or toggle the standalone clipboard window (default outside COSMIC panel)\n  -d, --delete         Open in delete mode (Enter removes the selected clip)\n  --applet             Run as a COSMIC panel applet\n  -V, --version        Print version information\n  -h, --help           Print this help message",
                env!("CARGO_PKG_VERSION")
            );
            return Ok(());
        }
        CliAction::Applet => {
            look::keep_fonts();
            spawn_applet_watcher_if_no_tray();
            return cosmic::applet::run::<Panel<false>>(LaunchFlags {
                window_mode: false,
                delete_mode: false,
            });
        }
        CliAction::Window { delete_mode } => {
            let runtime = tokio::runtime::Runtime::new().expect("D-Bus runtime");
            let offshot = std::env::var_os("MAPRISMA_OFFSHOT").is_some();
            let _connection = if offshot {
                None
            } else {
                let connection = runtime.block_on(async {
                    let connection = zbus::connection::Builder::session()?
                        .serve_at("/Window", WindowControl)?
                        .build()
                        .await?;
                    connection
                        .request_name_with_flags(WINDOW_BUS, zbus::fdo::RequestNameFlags::DoNotQueue.into())
                        .await?;
                    Ok::<_, zbus::Error>(connection)
                });
                match connection {
                    Ok(connection) => Some(connection),
                    Err(zbus::Error::NameTaken) => {
                        runtime.block_on(close_running_window(WINDOW_BUS));
                        return Ok(());
                    }
                    Err(error) => {
                        eprintln!("Window D-Bus: {error}");
                        std::process::exit(1);
                    }
                }
            };
            cosmic::app::run::<Panel<true>>(
                cosmic::app::Settings::default()
                    .default_font(look::default_font())
                    .size({
                        let (width, height) = window_start_size();
                        cosmic::iced::Size::new(width, height)
                    })
                    .resizable(Some(8.0))
                    .transparent(false)
                    .is_daemon(false),
                LaunchFlags {
                    window_mode: true,
                    delete_mode,
                },
            )
        }
    }
}

/// In COSMIC panel applet mode, runs the background clipboard watcher if the tray daemon isn't running.
fn spawn_applet_watcher_if_no_tray() {
    std::thread::spawn(|| {
        let Ok(rt) = tokio::runtime::Runtime::new() else { return };
        rt.block_on(async {
            let tray_running = if let Ok(conn) = zbus::Connection::session().await
                && let Ok(dbus) = zbus::fdo::DBusProxy::new(&conn).await
            {
                dbus.name_has_owner(TRAY_BUS.try_into().unwrap()).await.unwrap_or(false)
            } else {
                false
            };
            if !tray_running {
                let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
                clipboard::run_watcher(data_dir(), tx).await;
            }
        });
    });
}

fn window_start_size() -> window_size::Size {
    window_size::parse(i18n::read_pref(window_size::KEY).as_ref()).unwrap_or((POPUP_WIDTH, POPUP_HEIGHT))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EscStep {
    Note,
    Confirm,
    Menus,
    Back,
    ClearSearch,
    Close,
    Nothing,
}

#[derive(Debug, Default, Clone, Copy)]
struct EscState {
    shown: bool,
    note: bool,
    confirm: bool,
    menus: bool,
    settings: bool,
    has_query: bool,
}

fn esc_step(s: EscState) -> EscStep {
    if !s.shown {
        EscStep::Nothing
    } else if s.note {
        EscStep::Note
    } else if s.confirm {
        EscStep::Confirm
    } else if s.menus {
        EscStep::Menus
    } else if s.settings {
        EscStep::Back
    } else if s.has_query {
        EscStep::ClearSearch
    } else {
        EscStep::Close
    }
}

// ------------------------------------------------------------------ widgets & styles

fn dim_text(theme: &cosmic::Theme) -> iced_text::Style {
    let mut color = Color::from(theme.cosmic().on_bg_color());
    color.a = 0.62;
    iced_text::Style { color: Some(color), ..Default::default() }
}

fn accent_text(theme: &cosmic::Theme) -> iced_text::Style {
    iced_text::Style {
        color: Some(Color::from(theme.cosmic().accent_color())),
        ..Default::default()
    }
}

fn warn_text(theme: &cosmic::Theme) -> iced_text::Style {
    iced_text::Style {
        color: Some(Color::from(theme.cosmic().warning_color())),
        ..Default::default()
    }
}

fn dim<'a>(content: impl Into<std::borrow::Cow<'a, str>> + 'a) -> Element<'a, Message> {
    text::caption(content).class(theme::Text::Custom(dim_text)).into()
}

fn control_r() -> f32 {
    corners::now().control
}

fn small_r() -> f32 {
    corners::now().small
}

fn card_r() -> f32 {
    corners::now().radius
}

fn notice(warn: bool) -> theme::Container<'static> {
    theme::Container::custom(move |theme: &cosmic::Theme| {
        let color = if warn {
            Color::from(theme.cosmic().warning_color())
        } else {
            Color::from(theme.cosmic().accent_color())
        };
        let mut back = color;
        back.a = 0.13;
        let mut edge = color;
        edge.a = 0.50;
        iced_container::Style {
            background: Some(Background::Color(back)),
            border: Border {
                radius: control_r().into(),
                width: 1.0,
                color: edge,
            },
            ..Default::default()
        }
    })
}

fn card_box(selected: bool) -> theme::Container<'static> {
    theme::Container::custom(move |theme: &cosmic::Theme| {
        let c = theme.cosmic();
        if selected {
            let mut bg = Color::from(c.accent_color());
            bg.a = if c.is_dark { 0.16 } else { 0.12 };
            let mut edge = Color::from(c.accent_color());
            edge.a = 0.55;
            iced_container::Style {
                background: Some(Background::Color(bg)),
                border: Border {
                    radius: control_r().into(),
                    width: 1.0,
                    color: edge,
                },
                ..Default::default()
            }
        } else {
            let mut bg = Color::from(c.on_bg_color());
            bg.a = if c.is_dark { 0.04 } else { 0.03 };
            let mut edge = Color::from(c.on_bg_color());
            edge.a = 0.08;
            iced_container::Style {
                background: Some(Background::Color(bg)),
                border: Border {
                    radius: control_r().into(),
                    width: 1.0,
                    color: edge,
                },
                ..Default::default()
            }
        }
    })
}

fn inspector_box() -> theme::Container<'static> {
    theme::Container::custom(|theme: &cosmic::Theme| {
        let c = theme.cosmic();
        let mut bg = Color::from(c.on_bg_color());
        bg.a = if c.is_dark { 0.055 } else { 0.04 };
        let mut edge = Color::from(c.on_bg_color());
        edge.a = 0.14;
        iced_container::Style {
            background: Some(Background::Color(bg)),
            border: Border {
                radius: card_r().into(),
                width: 1.0,
                color: edge,
            },
            ..Default::default()
        }
    })
}

fn swatch_box(color: ColorRgba, radius: f32) -> theme::Container<'static> {
    theme::Container::custom(move |theme: &cosmic::Theme| {
        let mut edge = Color::from(theme.cosmic().on_bg_color());
        edge.a = 0.25;
        iced_container::Style {
            background: Some(Background::Color(Color::from_rgba8(
                color.r,
                color.g,
                color.b,
                color.a as f32 / 255.0,
            ))),
            border: Border {
                radius: radius.into(),
                width: 1.0,
                color: edge,
            },
            ..Default::default()
        }
    })
}

fn badge_pill(accent: bool) -> theme::Container<'static> {
    theme::Container::custom(move |theme: &cosmic::Theme| {
        let mut bg = if accent {
            Color::from(theme.cosmic().accent_color())
        } else {
            Color::from(theme.cosmic().on_bg_color())
        };
        bg.a = if accent { 0.18 } else { 0.08 };
        iced_container::Style {
            background: Some(Background::Color(bg)),
            border: Border {
                radius: small_r().into(),
                ..Default::default()
            },
            ..Default::default()
        }
    })
}

fn svg_glyph<'a>(bytes: &'static [u8], size: f32, alpha: f32, accent: bool) -> Element<'a, Message> {
    svg(iced_svg::Handle::from_memory(bytes))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .class(theme::Svg::custom(move |theme: &cosmic::Theme| {
            let mut c = if accent {
                Color::from(theme.cosmic().accent_color())
            } else {
                Color::from(theme.cosmic().on_bg_color())
            };
            c.a *= alpha;
            iced_svg::Style { color: Some(c) }
        }))
        .into()
}

fn tip<'a>(
    content: impl Into<Element<'a, Message>>,
    label: String,
    pos: cosmic::widget::tooltip::Position,
) -> Element<'a, Message> {
    cosmic::widget::tooltip(content, text::body(label), pos).into()
}

fn drop_button<'a>(label: String, open: bool, message: Option<Message>) -> Element<'a, Message> {
    button::text(label)
        .trailing_icon(icon::from_name(if open { "pan-up-symbolic" } else { "pan-down-symbolic" }).size(16))
        .on_press_maybe(message)
        .into()
}

fn list_item<'a>(label: String, chosen: bool, message: Message) -> Element<'a, Message> {
    let label = if chosen { format!("✓ {label}") } else { label };
    button::custom(text::body(label))
        .padding([6, 12])
        .width(Length::Fill)
        .class(theme::Button::MenuItem)
        .on_press(message)
        .into()
}

fn setting_row<'a>(label: impl Into<std::borrow::Cow<'a, str>> + 'a, control: Element<'a, Message>) -> Element<'a, Message> {
    row![text::body(label).width(Length::Fill), control]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
}

fn decode_thumbnail(path: &Path, max_w: u32, max_h: u32) -> Option<iced_image::Handle> {
    let img = image::open(path).ok()?;
    let thumb = img.thumbnail(max_w, max_h).to_rgba8();
    let (w, h) = thumb.dimensions();
    Some(iced_image::Handle::from_rgba(w, h, thumb.into_raw()))
}

// ------------------------------------------------------------------ state

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfirmClear {
    Unpinned,
    All,
}

struct UndoState {
    entry: ClipEntry,
    until: Instant,
}

#[derive(Default)]
struct Panel<const WINDOW_MODE: bool = false> {
    core: Core,
    window_mode: bool,
    delete_mode: bool,
    popup: Option<window::Id>,
    rectangle: Rectangle,
    tracker: Option<RectangleTracker<u32>>,
    dir: PathBuf,
    store: ClipboardStore,
    last_mtime: Option<SystemTime>,
    prefs: AppPrefs,
    query: String,
    filter: CategoryFilter,
    selected_id: Option<String>,
    last_click: Option<(String, Instant)>,
    thumbs: HashMap<String, iced_image::Handle>,
    previews: HashMap<String, iced_image::Handle>,
    undo: Option<UndoState>,
    confirm_clear: Option<ConfirmClear>,
    copied_toast_until: Option<Instant>,
    // Settings & F12
    settings_open: bool,
    lang_open: bool,
    corners_open: bool,
    limit_open: bool,
    corners: String,
    shot_key: bool,
    setting_error: bool,
    shot: shot::Phase,
    capturing: bool,
    resize: window_size::Debounce,
}

#[derive(Debug, Clone)]
enum Message {
    TogglePopup,
    CloseRequested(window::Id),
    Rectangle(RectangleUpdate<u32>),
    WindowTick,
    PollTick,
    SearchChanged(String),
    ClearSearch,
    SetFilter(CategoryFilter),
    SelectClip(String),
    CopyClip(String, bool),
    CopyDone(Result<(), String>, bool),
    TogglePin(String),
    DeleteClip(String),
    UndoDelete,
    OpenUrl(String),
    TogglePause,
    AskClear(ConfirmClear),
    ConfirmClearDo,
    CancelClear,
    ToggleSettings,
    ToggleLang,
    SetLanguage(String),
    ToggleCorners,
    SetCorners(String),
    ToggleLimit,
    SetLimit(usize),
    SetCloseOnCopy(bool),
    SetCaptureImages(bool),
    SetIgnoreSecrets(bool),
    SetShotKey(bool),
    KeyNavUp,
    KeyNavDown,
    KeyEnter(bool),
    KeyQuickCopy(usize),
    KeyPinSelected,
    KeyDeleteSelected(bool),
    ShotKey(window::Id),
    Escape(window::Id),
    Captured(window::Screenshot),
    ShotSaved(Result<shot::Saved, String>),
    NoteInput(String),
    NoteDone,
    NoteSaved(Result<(), String>),
}

fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) -> Message + Send + 'static,
) -> Task<Message> {
    let (sender, receiver) = cosmic::iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    Task::perform(receiver, move |result| match result {
        Ok(value) => cosmic::Action::App(done(value)),
        Err(_) => cosmic::Action::App(Message::PollTick),
    })
}

fn shot_key_setting() -> bool {
    i18n::read_pref("screenshot_key").and_then(|v| v.as_bool()).unwrap_or(true)
}

fn corners_setting() -> String {
    corners::normalize(i18n::read_pref("corners").as_ref().and_then(Value::as_str).unwrap_or("")).to_string()
}

impl<const WINDOW_MODE: bool> Panel<WINDOW_MODE> {
    fn shown(&self) -> bool {
        self.window_mode || self.popup.is_some()
    }

    fn visible_entries(&self) -> Vec<&ClipEntry> {
        self.store.filtered(&self.query, self.filter)
    }

    fn selected_entry(&self) -> Option<&ClipEntry> {
        let visible = self.visible_entries();
        if let Some(id) = &self.selected_id
            && let Some(found) = visible.iter().find(|e| &e.id == id)
        {
            return Some(*found);
        }
        visible.first().copied()
    }

    fn ensure_selection(&mut self) {
        let visible_ids: Vec<String> = self.visible_entries().into_iter().map(|e| e.id.clone()).collect();
        if visible_ids.is_empty() {
            self.selected_id = None;
            return;
        }
        if !self.selected_id.as_ref().is_some_and(|id| visible_ids.contains(id)) {
            self.selected_id = Some(visible_ids[0].clone());
        }
        self.ensure_selected_preview();
    }

    fn populate_thumbnails(&mut self) {
        let existing: Vec<(String, PathBuf)> = self
            .store
            .items
            .iter()
            .filter_map(|e| {
                if !self.thumbs.contains_key(&e.id) {
                    e.image_path(&self.dir).map(|p| (e.id.clone(), p))
                } else {
                    None
                }
            })
            .collect();
        for (id, path) in existing {
            if let Some(handle) = decode_thumbnail(&path, 88, 64) {
                self.thumbs.insert(id, handle);
            }
        }
        self.ensure_selected_preview();
    }

    fn ensure_selected_preview(&mut self) {
        if let Some(entry) = self.selected_entry()
            && !self.previews.contains_key(&entry.id)
            && let Some(path) = entry.image_path(&self.dir)
        {
            let id = entry.id.clone();
            if let Some(handle) = decode_thumbnail(&path, 420, 180) {
                self.previews.insert(id, handle);
            }
        }
    }

    fn reload_store(&mut self) {
        self.last_mtime = ClipboardStore::history_mtime(&self.dir);
        self.store = ClipboardStore::load_from(&self.dir);
        self.populate_thumbnails();
        self.ensure_selection();
    }

    fn save_store(&mut self) {
        if self.store.save_to(&self.dir).is_ok() {
            self.last_mtime = ClipboardStore::history_mtime(&self.dir);
        }
    }

    fn copy_entry(&mut self, id: &str, plain: bool) -> Task<Message> {
        let Some(entry) = self.store.items.iter().find(|e| e.id == id).cloned() else {
            return Task::none();
        };
        let dir = self.dir.clone();
        let should_close = self.prefs.close_on_copy;
        off_thread(
            move || match &entry.kind {
                ClipKind::Image { .. } => match entry.image_path(&dir) {
                    Some(path) => clipboard::copy_image(&path),
                    None => Err("Image file missing".into()),
                },
                _ => {
                    let payload = if plain { entry.plain_text() } else { entry.text };
                    clipboard::copy_text(&payload)
                }
            },
            move |res| Message::CopyDone(res, should_close),
        )
    }

    fn shutdown(&mut self) -> Task<Message> {
        self.save_size(true);
        if let shot::Phase::Note { path, text, .. } = std::mem::take(&mut self.shot) {
            let _ = shot::save_note(&path, SHOT_FOLDER, &text);
        }
        cosmic::iced::exit()
    }

    fn save_size(&mut self, closing: bool) {
        let sharp = self.core.window.sharp_corners;
        let ignore = || window_size::not_users_size(sharp);
        let size = if closing {
            self.resize.flush(ignore)
        } else {
            self.resize.due(Instant::now(), ignore)
        };
        if let Some(size) = size {
            self.setting_error = i18n::write_pref(window_size::KEY, Some(window_size::to_value(size))).is_err();
        }
        if closing && let Some(position) = window_size::current_position() {
            self.setting_error |=
                i18n::write_pref(window_size::POSITION_KEY, Some(window_size::position_value(position))).is_err();
        }
    }

    fn finish_note(&mut self, keep: bool) -> Task<Message> {
        match std::mem::take(&mut self.shot) {
            shot::Phase::Note { path, text, .. } => {
                if keep && shot::note_text(&text).is_some() {
                    self.shot = shot::Phase::Noting;
                    return off_thread(
                        move || shot::save_note(&path, SHOT_FOLDER, &text).map(|_| ()).map_err(|e| e.to_string()),
                        Message::NoteSaved,
                    );
                }
            }
            shot::Phase::Error(_) => {}
            other => self.shot = other,
        }
        Task::none()
    }

    fn shot_popup_closed(&mut self) -> Task<Message> {
        if std::mem::take(&mut self.capturing) && self.shot == shot::Phase::Busy {
            self.shot = shot::Phase::Idle;
        }
        if matches!(self.shot, shot::Phase::Note { .. }) {
            self.finish_note(true)
        } else {
            Task::none()
        }
    }

    fn popup_closed(&mut self) {
        self.lang_open = false;
        self.corners_open = false;
        self.limit_open = false;
        self.confirm_clear = None;
    }

    fn close_popup(&mut self) -> Task<Message> {
        if let Some(popup) = self.popup.take() {
            self.popup_closed();
            return Task::batch([self.shot_popup_closed(), destroy_popup(popup)]);
        }
        Task::none()
    }

    // ------------------------------------------------------------------ views

    fn shot_overlay(&self) -> Option<Element<'_, Message>> {
        let body: Element<Message> = match &self.shot {
            shot::Phase::Note { shown, text: note, .. } => column![
                text::heading(tr("Screenshot saved")),
                dim(trf("Saved as {0}", &[shown.clone()])),
                text_input(tr("What is wrong? (optional)"), note.as_str())
                    .id(NOTE_ID.clone())
                    .on_input(Message::NoteInput)
                    .on_submit(|_| Message::NoteDone),
                row![
                    space::horizontal().width(Length::Fill),
                    button::suggested(tr("Done")).on_press(Message::NoteDone),
                ],
            ]
            .spacing(12)
            .into(),
            shot::Phase::Error(reason) => column![
                text::body(reason.clone()),
                row![
                    space::horizontal().width(Length::Fill),
                    button::suggested(tr("Done")).on_press(Message::NoteDone),
                ],
            ]
            .spacing(12)
            .into(),
            _ => return None,
        };
        let card = container(body).padding(16).width(Length::Fill).class(theme::Container::Card);
        Some(
            container(card)
                .padding(12)
                .width(if self.window_mode { Length::Fill } else { Length::Fixed(POPUP_WIDTH) })
                .into(),
        )
    }

    fn header(&self) -> Element<'_, Message> {
        let count = self.store.items.len();
        let status_str = match (count, self.prefs.paused) {
            (0, false) => tr("Empty · Recording"),
            (0, true) => tr("Empty · Paused"),
            (1, false) => tr("1 item · Recording"),
            (1, true) => tr("1 item · Paused"),
            (n, false) => trf("{0} items · Recording", &[n.to_string()]),
            (n, true) => trf("{0} items · Paused", &[n.to_string()]),
        };
        let status_widget = if self.prefs.paused {
            text::caption_heading(status_str).class(theme::Text::Custom(warn_text))
        } else {
            text::caption(status_str).class(theme::Text::Custom(dim_text))
        };

        let pause_btn = tip(
            button::custom(svg_glyph(
                if self.prefs.paused { GLYPH_PLAY } else { GLYPH_PAUSE },
                16.0,
                1.0,
                self.prefs.paused,
            ))
            .padding(6)
            .class(theme::Button::Icon)
            .on_press(Message::TogglePause),
            tr(if self.prefs.paused { "Resume recording" } else { "Pause recording" }),
            cosmic::widget::tooltip::Position::Bottom,
        );

        let clear_btn = tip(
            button::custom(svg_glyph(GLYPH_TRASH, 16.0, 0.9, false))
                .padding(6)
                .class(theme::Button::Icon)
                .on_press_maybe((!self.store.items.is_empty()).then_some(Message::AskClear(ConfirmClear::Unpinned))),
            tr("Clear unpinned"),
            cosmic::widget::tooltip::Position::Bottom,
        );

        let settings_btn = tip(
            button::icon(icon::from_name("emblem-system-symbolic").symbolic(true))
                .on_press(Message::ToggleSettings),
            tr("Settings"),
            cosmic::widget::tooltip::Position::Bottom,
        );

        row![
            svg_glyph(
                if self.prefs.paused { GLYPH_PAUSED } else { GLYPH_CLIPBOARD },
                28.0,
                if self.prefs.paused { 0.7 } else { 1.0 },
                false,
            ),
            column![text::title4(tr("Maprisma Clipboard")), status_widget].spacing(2),
            space::horizontal().width(Length::Fill),
            pause_btn,
            clear_btn,
            settings_btn,
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    fn search_bar(&self) -> Element<'_, Message> {
        let input = text_input(tr("Search clipboard history…"), self.query.as_str())
            .id(SEARCH_ID.clone())
            .on_input(Message::SearchChanged)
            .on_submit(|_| Message::KeyEnter(false));

        if self.query.is_empty() {
            input.into()
        } else {
            row![
                input,
                tip(
                    button::custom(svg_glyph(GLYPH_CLOSE, 14.0, 0.8, false))
                        .padding(6)
                        .class(theme::Button::Icon)
                        .on_press(Message::ClearSearch),
                    tr("Clear search"),
                    cosmic::widget::tooltip::Position::Bottom,
                ),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .into()
        }
    }

    fn filter_row(&self) -> Element<'_, Message> {
        let mut chips: Vec<Element<Message>> = Vec::with_capacity(CategoryFilter::ALL.len());
        for f in CategoryFilter::ALL {
            let active = self.filter == f;
            let count = self.store.count_for(f);
            let label = if f == CategoryFilter::All || count == 0 {
                tr(f.label_key())
            } else {
                format!("{} {count}", tr(f.label_key()))
            };
            let btn = if active {
                button::suggested(label)
            } else {
                button::standard(label)
            };
            chips.push(btn.padding([4, 8]).on_press(Message::SetFilter(f)).into());
        }
        flex_row(chips).spacing(6).into()
    }

    fn confirm_banner(&self) -> Option<Element<'_, Message>> {
        let kind = self.confirm_clear?;
        let (title, subtitle) = match kind {
            ConfirmClear::Unpinned => (tr("Clear all unpinned items?"), tr("Pinned items will be kept.")),
            ConfirmClear::All => (
                tr("Clear entire clipboard history including pinned items?"),
                tr("This cannot be undone."),
            ),
        };
        let body = column![
            text::heading(title),
            dim(subtitle),
            row![
                space::horizontal().width(Length::Fill),
                button::standard(tr("Cancel")).on_press(Message::CancelClear),
                button::destructive(tr("Confirm")).on_press(Message::ConfirmClearDo),
            ]
            .spacing(8),
        ]
        .spacing(8);
        Some(container(body).padding(12).width(Length::Fill).class(notice(true)).into())
    }

    fn undo_banner(&self) -> Option<Element<'_, Message>> {
        self.undo.as_ref()?;
        let bar = row![
            text::body(tr("Removed clip")).width(Length::Fill),
            button::suggested(tr("Undo")).padding([4, 12]).on_press(Message::UndoDelete),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        Some(container(bar).padding([8, 12]).width(Length::Fill).class(notice(false)).into())
    }

    fn clip_meta(&self, entry: &ClipEntry, now: u64) -> String {
        let age = relative_time(entry.timestamp, now);
        match &entry.kind {
            ClipKind::Color(c) => format!("{} · {} · {age}", tr("Color"), c.hex()),
            ClipKind::Image { width, height, bytes, .. } => {
                format!("{} · {width}×{height} · {} · {age}", tr("Image"), format_bytes(*bytes))
            }
            ClipKind::Link => {
                let dom = entry.url_domain().unwrap_or_default();
                if dom.is_empty() {
                    format!("{} · {age}", tr("Link"))
                } else {
                    format!("{} · {dom} · {age}", tr("Link"))
                }
            }
            ClipKind::Code => {
                let lines = if entry.lines == 1 {
                    tr("1 line")
                } else {
                    trf("{0} lines", &[entry.lines.to_string()])
                };
                format!("{} · {lines} · {age}", tr("Code"))
            }
            ClipKind::Email => format!("{} · {age}", tr("Email")),
            ClipKind::Text => {
                let chars = trf("{0} chars", &[entry.chars.to_string()]);
                if entry.lines > 1 {
                    let lines = trf("{0} lines", &[entry.lines.to_string()]);
                    format!("{} · {chars} · {lines} · {age}", tr("Text"))
                } else {
                    format!("{} · {chars} · {age}", tr("Text"))
                }
            }
        }
    }

    fn clip_leading<'a>(&'a self, entry: &'a ClipEntry) -> Element<'a, Message> {
        match &entry.kind {
            ClipKind::Color(c) => container(space::horizontal())
                .width(Length::Fixed(30.0))
                .height(Length::Fixed(30.0))
                .class(swatch_box(*c, small_r()))
                .into(),
            ClipKind::Image { .. } => {
                if let Some(handle) = self.thumbs.get(&entry.id) {
                    container(
                        cosmic_image::Image::new(handle.clone())
                            .width(Length::Fixed(42.0))
                            .height(Length::Fixed(30.0))
                            .content_fit(ContentFit::Cover),
                    )
                    .class(badge_pill(false))
                    .into()
                } else {
                    container(svg_glyph(GLYPH_CLIPBOARD, 16.0, 0.85, false))
                        .width(Length::Fixed(30.0))
                        .height(Length::Fixed(30.0))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center)
                        .class(badge_pill(false))
                        .into()
                }
            }
            ClipKind::Link => container(svg_glyph(GLYPH_LINK, 16.0, 1.0, true))
                .width(Length::Fixed(30.0))
                .height(Length::Fixed(30.0))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
                .class(badge_pill(true))
                .into(),
            ClipKind::Code => container(svg_glyph(GLYPH_CODE, 16.0, 0.9, false))
                .width(Length::Fixed(30.0))
                .height(Length::Fixed(30.0))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
                .class(badge_pill(false))
                .into(),
            ClipKind::Email => container(svg_glyph(GLYPH_MAIL, 16.0, 0.9, false))
                .width(Length::Fixed(30.0))
                .height(Length::Fixed(30.0))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
                .class(badge_pill(false))
                .into(),
            ClipKind::Text => container(svg_glyph(GLYPH_TEXT, 16.0, 0.75, false))
                .width(Length::Fixed(30.0))
                .height(Length::Fixed(30.0))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
                .class(badge_pill(false))
                .into(),
        }
    }

    fn clip_row<'a>(&'a self, idx: usize, entry: &'a ClipEntry, selected: bool, now: u64) -> Element<'a, Message> {
        let index_badge: Element<Message> = if idx < 9 {
            text::caption(format!("{}", idx + 1))
                .font(look::mono_font())
                .width(Length::Fixed(14.0))
                .class(theme::Text::Custom(dim_text))
                .into()
        } else {
            space::horizontal().width(Length::Fixed(14.0)).into()
        };

        let preview_text = entry.preview_line(56);
        let primary: Element<Message> = match &entry.kind {
            ClipKind::Code | ClipKind::Color(_) => text::body(preview_text).font(look::mono_font()).into(),
            ClipKind::Link => {
                if let Some(dom) = entry.url_domain() {
                    let rest = entry
                        .text
                        .trim()
                        .strip_prefix("https://")
                        .or_else(|| entry.text.trim().strip_prefix("http://"))
                        .unwrap_or("");
                    let path = rest.find('/').map(|i| &rest[i..]).unwrap_or("");
                    let mut r = row![text::body(dom).class(theme::Text::Custom(accent_text))]
                        .spacing(6)
                        .align_y(Alignment::Center);
                    if !path.is_empty() && path != "/" {
                        r = r.push(dim(path.chars().take(28).collect::<String>()));
                    }
                    r.into()
                } else {
                    text::body(preview_text).into()
                }
            }
            _ => text::body(preview_text).into(),
        };

        let meta_line = if entry.pinned {
            row![
                svg_glyph(GLYPH_PIN, 11.0, 1.0, true),
                dim(self.clip_meta(entry, now)),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
        } else {
            row![dim(self.clip_meta(entry, now))]
        };

        let id = entry.id.clone();
        let pin_btn = tip(
            button::custom(svg_glyph(GLYPH_PIN, 14.0, if entry.pinned { 1.0 } else { 0.55 }, entry.pinned))
                .padding(4)
                .class(theme::Button::Icon)
                .on_press(Message::TogglePin(id.clone())),
            tr(if entry.pinned { "Unpin" } else { "Pin" }),
            cosmic::widget::tooltip::Position::Top,
        );
        let copy_btn = tip(
            button::custom(svg_glyph(GLYPH_COPY, 14.0, 0.85, false))
                .padding(4)
                .class(theme::Button::Icon)
                .on_press(Message::CopyClip(id.clone(), false)),
            tr("Copy"),
            cosmic::widget::tooltip::Position::Top,
        );
        let del_btn = tip(
            button::custom(svg_glyph(GLYPH_TRASH, 14.0, 0.7, false))
                .padding(4)
                .class(theme::Button::Icon)
                .on_press(Message::DeleteClip(id.clone())),
            tr("Delete"),
            cosmic::widget::tooltip::Position::Top,
        );

        let left_area = button::custom(
            row![
                index_badge,
                self.clip_leading(entry),
                column![primary, meta_line].spacing(2).width(Length::Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .padding([6, 6])
        .width(Length::Fill)
        .class(theme::Button::MenuItem)
        .on_press(Message::SelectClip(id));

        let full_row = row![left_area, pin_btn, copy_btn, del_btn]
            .spacing(2)
            .align_y(Alignment::Center);

        container(full_row)
            .padding([2, 6])
            .width(Length::Fill)
            .class(card_box(selected))
            .into()
    }

    fn clip_list(&self) -> Element<'_, Message> {
        let visible = self.visible_entries();
        if visible.is_empty() {
            let (headline, hint, action): (String, String, Option<Element<Message>>) = if self.store.items.is_empty() {
                if self.prefs.paused {
                    (
                        tr("Recording is paused"),
                        tr("Resume recording to capture new clips."),
                        Some(button::suggested(tr("Resume recording")).on_press(Message::TogglePause).into()),
                    )
                } else {
                    (
                        tr("No clipboard items yet"),
                        tr("Copy text, links, colors or images and they will appear here."),
                        None,
                    )
                }
            } else {
                (
                    tr("No matching clips"),
                    tr("Try a different search term or filter."),
                    Some(
                        button::standard(tr("Clear search"))
                            .on_press(Message::ClearSearch)
                            .into(),
                    ),
                )
            };
            let mut empty = column![text::heading(headline), dim(hint)]
                .spacing(6)
                .align_x(Alignment::Center);
            if let Some(btn) = action {
                empty = empty.push(space::vertical().height(Length::Fixed(4.0))).push(btn);
            }
            return container(empty)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
                .padding(24)
                .into();
        }

        let now = now_unix();
        let selected_id = self.selected_entry().map(|e| e.id.as_str());
        let mut list = column![].spacing(6);
        for (idx, entry) in visible.iter().enumerate() {
            let is_sel = selected_id == Some(entry.id.as_str());
            list = list.push(self.clip_row(idx, entry, is_sel, now));
        }

        scrollable(container(list).padding([0, 10, 0, 0]).width(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn inspector(&self) -> Option<Element<'_, Message>> {
        let entry = self.selected_entry()?;
        let body: Element<Message> = match &entry.kind {
            ClipKind::Color(c) => row![
                container(space::horizontal())
                    .width(Length::Fixed(52.0))
                    .height(Length::Fixed(52.0))
                    .class(swatch_box(*c, control_r())),
                column![
                    text::heading(c.hex()).font(look::mono_font()),
                    text::caption(c.rgb_string()).font(look::mono_font()).class(theme::Text::Custom(dim_text)),
                    text::caption(c.hsl_string()).font(look::mono_font()).class(theme::Text::Custom(dim_text)),
                ]
                .spacing(2)
                .width(Length::Fill),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
            ClipKind::Image { width, height, bytes, .. } => {
                let mut col = column![].spacing(6).align_x(Alignment::Center);
                if let Some(handle) = self.previews.get(&entry.id).or_else(|| self.thumbs.get(&entry.id)) {
                    col = col.push(
                        cosmic_image::Image::new(handle.clone())
                            .width(Length::Fill)
                            .height(Length::Fixed(104.0))
                            .content_fit(ContentFit::Contain),
                    );
                }
                col = col.push(dim(format!("{width} × {height} px · {}", format_bytes(*bytes))));
                col.into()
            }
            ClipKind::Code => {
                let snippet: String = entry.text.chars().take(1200).collect();
                scrollable(text::caption(snippet).font(look::mono_font()))
                    .width(Length::Fill)
                    .height(Length::Fixed(78.0))
                    .into()
            }
            ClipKind::Link => column![
                text::heading(entry.url_domain().unwrap_or_else(|| tr("Link")))
                    .class(theme::Text::Custom(accent_text)),
                text::caption(entry.text.trim().chars().take(240).collect::<String>())
                    .class(theme::Text::Custom(dim_text)),
            ]
            .spacing(4)
            .into(),
            ClipKind::Text | ClipKind::Email => {
                let snippet: String = entry.text.chars().take(900).collect();
                scrollable(text::body(snippet))
                    .width(Length::Fill)
                    .height(Length::Fixed(72.0))
                    .into()
            }
        };

        let id = entry.id.clone();
        let mut actions = row![].spacing(6).align_y(Alignment::Center);

        actions = actions.push(
            button::standard(tr(if entry.pinned { "Unpin" } else { "Pin" }))
                .padding([5, 10])
                .on_press(Message::TogglePin(id.clone())),
        );

        match &entry.kind {
            ClipKind::Link => {
                actions = actions.push(
                    button::standard(tr("Open URL"))
                        .padding([5, 10])
                        .on_press(Message::OpenUrl(entry.text.trim().to_string())),
                );
            }
            ClipKind::Text | ClipKind::Code if entry.lines > 1 => {
                actions = actions.push(
                    button::standard(tr("Plain text"))
                        .padding([5, 10])
                        .on_press(Message::CopyClip(id.clone(), true)),
                );
            }
            _ => {}
        }

        actions = actions.push(
            button::destructive(tr("Delete"))
                .padding([5, 10])
                .on_press(Message::DeleteClip(id.clone())),
        );

        actions = actions.push(space::horizontal().width(Length::Fill));

        let copy_label = if self.copied_toast_until.is_some_and(|t| Instant::now() < t) {
            tr("Copied")
        } else {
            tr("Copy")
        };
        actions = actions.push(
            button::suggested(copy_label)
                .padding([5, 14])
                .on_press(Message::CopyClip(id, false)),
        );

        let card = column![body, divider::horizontal::default(), actions].spacing(10);
        Some(container(card).padding(12).width(Length::Fill).class(inspector_box()).into())
    }

    fn settings_page(&self) -> Element<'_, Message> {
        let header = row![
            button::text(tr("Back"))
                .leading_icon(icon::from_name("go-previous-symbolic").size(16))
                .on_press(Message::ToggleSettings),
            space::horizontal().width(Length::Fill),
            text::caption_heading(tr("SETTINGS")).class(theme::Text::Custom(dim_text)),
        ]
        .align_y(Alignment::Center);

        let mut page = column![header, divider::horizontal::default()].spacing(10);

        // Language
        let lang_choices = i18n::choices();
        let cur_lang = i18n::setting();
        let cur_lang_label = lang_choices
            .iter()
            .find(|(c, _)| *c == cur_lang)
            .map(|(_, l)| l.clone())
            .unwrap_or_default();
        page = page.push(setting_row(
            tr("Language"),
            drop_button(cur_lang_label, self.lang_open, Some(Message::ToggleLang)),
        ));
        if self.lang_open {
            let mut list = column![];
            for (code, label) in lang_choices {
                list = list.push(list_item(label, cur_lang == code, Message::SetLanguage(code)));
            }
            page = page.push(list);
        }

        // Corners
        let desktop_px = corners::desktop_radius().round().to_string();
        let corner_choices = [
            ("desktop", trf("Like the desktop ({0} px)", &[desktop_px])),
            ("slight", tr("Slightly rounded (6 px)")),
            ("rounded", tr("Rounded (12 px)")),
            ("square", tr("Square")),
        ];
        let cur_corners = corner_choices
            .iter()
            .find(|(c, _)| *c == self.corners)
            .map(|(_, l)| l.clone())
            .unwrap_or_default();
        page = page.push(setting_row(
            tr("Corners"),
            drop_button(cur_corners, self.corners_open, Some(Message::ToggleCorners)),
        ));
        if self.corners_open {
            let mut list = column![];
            for (choice, label) in corner_choices {
                list = list.push(list_item(label, self.corners == choice, Message::SetCorners(choice.into())));
            }
            page = page.push(list);
        }

        // F12 screenshot
        page = page.push(setting_row(
            tr("F12 saves a screenshot"),
            toggler(self.shot_key).on_toggle(Message::SetShotKey).into(),
        ));

        page = page.push(divider::horizontal::default());

        // History limit
        let cur_limit_label = trf("{0} items", &[self.prefs.max_items.to_string()]);
        page = page.push(setting_row(
            tr("History limit"),
            drop_button(cur_limit_label, self.limit_open, Some(Message::ToggleLimit)),
        ));
        if self.limit_open {
            let mut list = column![];
            for limit in HISTORY_LIMITS {
                let lbl = trf("{0} items", &[limit.to_string()]);
                list = list.push(list_item(lbl, self.prefs.max_items == limit, Message::SetLimit(limit)));
            }
            page = page.push(list);
        }

        // Behavior toggles
        page = page.push(setting_row(
            tr("Close window after copying"),
            toggler(self.prefs.close_on_copy).on_toggle(Message::SetCloseOnCopy).into(),
        ));
        page = page.push(setting_row(
            tr("Capture images"),
            toggler(self.prefs.capture_images).on_toggle(Message::SetCaptureImages).into(),
        ));
        page = page.push(setting_row(
            tr("Ignore password manager secrets"),
            toggler(self.prefs.ignore_secrets).on_toggle(Message::SetIgnoreSecrets).into(),
        ));

        page = page.push(divider::horizontal::default());

        if let Some(confirm) = self.confirm_banner() {
            page = page.push(confirm);
        }

        // Destructive actions
        page = page.push(
            row![
                button::standard(tr("Clear unpinned"))
                    .on_press_maybe((!self.store.items.is_empty()).then_some(Message::AskClear(ConfirmClear::Unpinned))),
                space::horizontal().width(Length::Fill),
                button::destructive(tr("Clear all history"))
                    .on_press_maybe((!self.store.items.is_empty()).then_some(Message::AskClear(ConfirmClear::All))),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );

        if self.setting_error {
            page = page.push(dim(tr("Could not save the setting.")));
        }

        scrollable(container(page).padding([12, 16]))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn main_page(&self) -> Element<'_, Message> {
        let mut col = column![self.header(), divider::horizontal::default()].spacing(10);

        if self.delete_mode {
            col = col.push(
                container(text::caption(tr("Delete mode — press Enter or Delete to remove the selected clip")))
                    .padding([8, 12])
                    .width(Length::Fill)
                    .class(notice(true)),
            );
        }

        if let Some(confirm) = self.confirm_banner() {
            col = col.push(confirm);
        }

        if let Some(undo) = self.undo_banner() {
            col = col.push(undo);
        }

        col = col.push(self.search_bar());
        col = col.push(self.filter_row());
        col = col.push(self.clip_list());

        if let Some(insp) = self.inspector() {
            col = col.push(insp);
        }

        container(col)
            .padding([12, 16])
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn popup_content(&self) -> Element<'_, Message> {
        if let Some(overlay) = self.shot_overlay() {
            return overlay;
        }
        let body = if self.settings_open {
            self.settings_page()
        } else {
            self.main_page()
        };
        if self.window_mode {
            container(body).width(Length::Fill).height(Length::Fill).into()
        } else {
            container(body)
                .width(Length::Fixed(POPUP_WIDTH))
                .height(Length::Fixed(POPUP_MAX_HEIGHT.min(660.0)))
                .into()
        }
    }
}

impl<const WINDOW_MODE: bool> cosmic::Application for Panel<WINDOW_MODE> {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = LaunchFlags;
    type Message = Message;
    const APP_ID: &'static str = if WINDOW_MODE { WINDOW_APP_ID } else { APP_ID };

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, flags: LaunchFlags) -> (Self, Task<Message>) {
        let mut core = core;
        look::solid(&mut core);
        let dir = data_dir();
        let prefs = load_prefs();
        let store = ClipboardStore::load_or_migrate(&dir, prefs.max_items);
        let last_mtime = ClipboardStore::history_mtime(&dir);
        let corners_val = corners_setting();

        let mut app = Self {
            core,
            window_mode: flags.window_mode,
            delete_mode: flags.delete_mode,
            dir,
            store,
            last_mtime,
            prefs,
            shot_key: shot_key_setting(),
            corners: corners_val.clone(),
            ..Default::default()
        };

        // Allow MAPRISMA_SHOT_PAGE=settings during debug or offshot capture
        let allow_shot_page = cfg!(debug_assertions) || std::env::var_os("MAPRISMA_OFFSHOT").is_some();
        if allow_shot_page && std::env::var("MAPRISMA_SHOT_PAGE").as_deref() == Ok("settings") {
            app.settings_open = true;
        }

        app.populate_thumbnails();
        app.ensure_selection();

        if flags.window_mode {
            app.resize = window_size::Debounce::new(window_start_size());
            window_size::restore_position(
                window_size::parse_position(i18n::read_pref(window_size::POSITION_KEY).as_ref()),
                window_start_size(),
            );
        }
        corners::set_current(&app.corners);

        let mut tasks = Vec::new();
        if flags.window_mode {
            app.set_header_title(APP_NAME.into());
            if let Some(id) = app.core.main_window_id() {
                tasks.push(app.set_window_title(APP_NAME.into(), id));
            }
            tasks.push(corners::set_app_theme(&app.corners));
            if !app.settings_open {
                tasks.push(text_input::focus(SEARCH_ID.clone()));
            }
        }
        (app, Task::batch(tasks))
    }

    fn system_theme_update(&mut self, _keys: &[&'static str], new_theme: &cosmic::cosmic_theme::Theme) -> Task<Message> {
        corners::system_theme_update(&self.corners, new_theme, self.window_mode)
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::CloseRequested(id))
    }

    fn on_window_resize(&mut self, id: window::Id, width: f32, height: f32) {
        if self.window_mode && self.core.main_window_id() == Some(id) {
            self.resize.resized(width, height, Instant::now());
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = Vec::new();
        if !self.window_mode {
            subs.push(rectangle_tracker_subscription(0).map(|u| Message::Rectangle(u.1)));
        } else {
            subs.push(cosmic::iced::time::every(Duration::from_millis(250)).map(|_| Message::WindowTick));
        }
        if self.shown() {
            subs.push(cosmic::iced::time::every(Duration::from_millis(500)).map(|_| Message::PollTick));
        }
        subs.push(cosmic::iced::event::listen_with(|event, status, id| match event {
            cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                if shot::is_shot_key(&key, modifiers) {
                    return Some(Message::ShotKey(id));
                }
                if matches!(key, keyboard::Key::Named(keyboard::key::Named::Escape)) {
                    return Some(Message::Escape(id));
                }
                match &key {
                    keyboard::Key::Named(keyboard::key::Named::ArrowUp) => Some(Message::KeyNavUp),
                    keyboard::Key::Named(keyboard::key::Named::ArrowDown) => Some(Message::KeyNavDown),
                    keyboard::Key::Named(keyboard::key::Named::Enter) => Some(Message::KeyEnter(modifiers.shift())),
                    keyboard::Key::Named(keyboard::key::Named::Delete) => {
                        if modifiers.control() {
                            Some(Message::KeyDeleteSelected(true))
                        } else if status == cosmic::iced::event::Status::Ignored {
                            Some(Message::KeyDeleteSelected(false))
                        } else {
                            None
                        }
                    }
                    keyboard::Key::Character(c) if modifiers.control() => {
                        let s = c.as_str();
                        if s.eq_ignore_ascii_case("p") {
                            Some(Message::KeyPinSelected)
                        } else if let Some(d) = s.chars().next().filter(|ch| ('1'..='9').contains(ch)) {
                            Some(Message::KeyQuickCopy((d as u8 - b'1') as usize))
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }));
        Subscription::batch(subs)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SearchChanged(q) => {
                self.query = q;
                self.ensure_selection();
            }
            Message::ClearSearch => {
                self.query.clear();
                self.filter = CategoryFilter::All;
                self.ensure_selection();
                return text_input::focus(SEARCH_ID.clone());
            }
            Message::SetFilter(f) => {
                self.filter = f;
                self.ensure_selection();
            }
            Message::SelectClip(id) => {
                let now = Instant::now();
                let is_double = self
                    .last_click
                    .as_ref()
                    .is_some_and(|(prev, at)| prev == &id && now.duration_since(*at) < Duration::from_millis(400));
                self.selected_id = Some(id.clone());
                self.last_click = Some((id.clone(), now));
                self.ensure_selected_preview();
                if is_double {
                    return self.copy_entry(&id, false);
                }
            }
            Message::CopyClip(id, plain) => {
                self.selected_id = Some(id.clone());
                return self.copy_entry(&id, plain);
            }
            Message::CopyDone(res, should_close) => {
                if res.is_ok() {
                    if should_close {
                        if self.window_mode {
                            return self.shutdown();
                        }
                        return self.close_popup();
                    }
                    self.copied_toast_until = Some(Instant::now() + Duration::from_millis(1500));
                }
            }
            Message::TogglePin(id) => {
                if self.store.toggle_pin(&id) {
                    self.save_store();
                    self.ensure_selection();
                }
            }
            Message::DeleteClip(id) => {
                if let Some(removed) = self.store.remove(&id) {
                    self.undo = Some(UndoState {
                        entry: removed,
                        until: Instant::now() + Duration::from_secs(5),
                    });
                    self.save_store();
                    self.ensure_selection();
                }
            }
            Message::UndoDelete => {
                if let Some(undo) = self.undo.take() {
                    let id = undo.entry.id.clone();
                    self.store.restore(undo.entry);
                    self.save_store();
                    self.selected_id = Some(id);
                    self.populate_thumbnails();
                }
            }
            Message::OpenUrl(url) => {
                if url.starts_with("http://") || url.starts_with("https://") {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(url)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                }
            }
            Message::TogglePause => {
                self.prefs.paused = !self.prefs.paused;
                self.setting_error = i18n::write_pref("paused", Some(self.prefs.paused.into())).is_err();
            }
            Message::AskClear(kind) => {
                self.confirm_clear = Some(kind);
            }
            Message::CancelClear => {
                self.confirm_clear = None;
            }
            Message::ConfirmClearDo => {
                if let Some(kind) = self.confirm_clear.take() {
                    match kind {
                        ConfirmClear::Unpinned => self.store.clear_unpinned(Some(&self.dir)),
                        ConfirmClear::All => self.store.clear_all(Some(&self.dir)),
                    }
                    self.undo = None;
                    self.thumbs.clear();
                    self.previews.clear();
                    self.save_store();
                    self.populate_thumbnails();
                    self.ensure_selection();
                }
            }
            Message::ToggleSettings => {
                self.settings_open = !self.settings_open;
                self.lang_open = false;
                self.corners_open = false;
                self.limit_open = false;
                self.confirm_clear = None;
                if !self.settings_open {
                    return text_input::focus(SEARCH_ID.clone());
                }
            }
            Message::ToggleLang => {
                self.lang_open = !self.lang_open;
                self.corners_open = false;
                self.limit_open = false;
            }
            Message::SetLanguage(code) => {
                self.setting_error = i18n::save(&code).is_err();
                self.lang_open = false;
            }
            Message::ToggleCorners => {
                self.corners_open = !self.corners_open;
                self.lang_open = false;
                self.limit_open = false;
            }
            Message::SetCorners(choice) => {
                let choice = corners::normalize(&choice);
                let value = (choice != "desktop").then(|| Value::from(choice));
                self.setting_error = i18n::write_pref("corners", value).is_err();
                self.corners = choice.to_string();
                corners::set_current(choice);
                self.corners_open = false;
                if self.window_mode {
                    return corners::set_app_theme(choice);
                }
            }
            Message::ToggleLimit => {
                self.limit_open = !self.limit_open;
                self.lang_open = false;
                self.corners_open = false;
            }
            Message::SetLimit(limit) => {
                self.prefs.max_items = limit;
                self.setting_error = i18n::write_pref("max_items", Some((limit as u64).into())).is_err();
                self.limit_open = false;
                self.store.prune(limit, Some(&self.dir));
                self.save_store();
                self.ensure_selection();
            }
            Message::SetCloseOnCopy(on) => {
                self.prefs.close_on_copy = on;
                self.setting_error = i18n::write_pref("close_on_copy", Some(on.into())).is_err();
            }
            Message::SetCaptureImages(on) => {
                self.prefs.capture_images = on;
                self.setting_error = i18n::write_pref("capture_images", Some(on.into())).is_err();
            }
            Message::SetIgnoreSecrets(on) => {
                self.prefs.ignore_secrets = on;
                self.setting_error = i18n::write_pref("ignore_secrets", Some(on.into())).is_err();
            }
            Message::SetShotKey(on) => {
                self.shot_key = on;
                self.setting_error = i18n::write_pref("screenshot_key", Some(on.into())).is_err();
            }
            Message::KeyNavUp => {
                if self.settings_open || self.shot.open() {
                    return Task::none();
                }
                let ids: Vec<String> = self.visible_entries().into_iter().map(|e| e.id.clone()).collect();
                if !ids.is_empty() {
                    let cur = self
                        .selected_id
                        .as_ref()
                        .and_then(|id| ids.iter().position(|x| x == id))
                        .unwrap_or(0);
                    let next = cur.saturating_sub(1);
                    self.selected_id = Some(ids[next].clone());
                    self.ensure_selected_preview();
                }
            }
            Message::KeyNavDown => {
                if self.settings_open || self.shot.open() {
                    return Task::none();
                }
                let ids: Vec<String> = self.visible_entries().into_iter().map(|e| e.id.clone()).collect();
                if !ids.is_empty() {
                    let cur = self
                        .selected_id
                        .as_ref()
                        .and_then(|id| ids.iter().position(|x| x == id))
                        .unwrap_or(0);
                    let next = (cur + 1).min(ids.len() - 1);
                    self.selected_id = Some(ids[next].clone());
                    self.ensure_selected_preview();
                }
            }
            Message::KeyEnter(plain) => {
                if self.settings_open || self.shot.open() {
                    return Task::none();
                }
                if let Some(entry) = self.selected_entry() {
                    let id = entry.id.clone();
                    if self.delete_mode {
                        return self.update(Message::DeleteClip(id));
                    }
                    return self.copy_entry(&id, plain);
                }
            }
            Message::KeyQuickCopy(idx) => {
                if self.settings_open || self.shot.open() {
                    return Task::none();
                }
                let id = self.visible_entries().get(idx).map(|e| e.id.clone());
                if let Some(id) = id {
                    return self.copy_entry(&id, false);
                }
            }
            Message::KeyPinSelected => {
                if self.settings_open || self.shot.open() {
                    return Task::none();
                }
                if let Some(entry) = self.selected_entry() {
                    let id = entry.id.clone();
                    return self.update(Message::TogglePin(id));
                }
            }
            Message::KeyDeleteSelected(forced_ctrl) => {
                if self.settings_open || self.shot.open() {
                    return Task::none();
                }
                if (forced_ctrl || self.query.is_empty())
                    && let Some(entry) = self.selected_entry()
                {
                    let id = entry.id.clone();
                    return self.update(Message::DeleteClip(id));
                }
            }
            Message::PollTick => {
                if self.undo.as_ref().is_some_and(|u| Instant::now() >= u.until) {
                    self.undo = None;
                    self.store.cleanup_orphan_images(&self.dir);
                }
                if self.copied_toast_until.is_some_and(|t| Instant::now() >= t) {
                    self.copied_toast_until = None;
                }
                i18n::reload_prefs();
                self.prefs = load_prefs();
                let mtime = ClipboardStore::history_mtime(&self.dir);
                if mtime != self.last_mtime {
                    self.reload_store();
                }
            }
            Message::WindowTick => {
                if CLOSE_WINDOW.swap(false, Ordering::Relaxed) {
                    return self.shutdown();
                }
                if self.resize.waiting() {
                    self.save_size(false);
                }
                if look::refresh() {
                    return corners::set_app_theme(&self.corners);
                }
            }
            Message::ShotKey(id) => {
                let shown = if self.window_mode {
                    self.core.main_window_id() == Some(id)
                } else {
                    self.popup == Some(id)
                };
                if self.shot_key && shown && self.shot == shot::Phase::Idle {
                    self.shot = shot::Phase::Busy;
                    self.capturing = true;
                    return window::screenshot(id).map(|p| cosmic::Action::App(Message::Captured(p)));
                }
            }
            Message::Captured(picture) => {
                if !std::mem::take(&mut self.capturing) {
                    return Task::none();
                }
                let (width, height) = (picture.size.width, picture.size.height);
                return off_thread(
                    move || shot::save_screenshot(SHOT_FOLDER, &picture.rgba, width, height),
                    Message::ShotSaved,
                );
            }
            Message::ShotSaved(result) => {
                self.shot = match result {
                    Ok(saved) => shot::Phase::Note {
                        path: saved.path,
                        shown: saved.shown,
                        text: String::new(),
                    },
                    Err(reason) => shot::Phase::Error(trf("The screenshot could not be saved: {0}", &[reason])),
                };
                if matches!(self.shot, shot::Phase::Note { .. }) {
                    return text_input::focus(NOTE_ID.clone());
                }
            }
            Message::NoteInput(value) => {
                if let shot::Phase::Note { text, .. } = &mut self.shot {
                    *text = value.chars().take(shot::NOTE_LIMIT).collect();
                }
            }
            Message::NoteDone => return self.finish_note(true),
            Message::NoteSaved(result) => match result {
                Ok(()) => {
                    if self.shot == shot::Phase::Noting {
                        self.shot = shot::Phase::Idle;
                    }
                }
                Err(reason) => {
                    self.shot = shot::Phase::Error(trf("The note could not be saved: {0}", &[reason]));
                }
            },
            Message::Escape(id) => {
                let shown = if self.window_mode {
                    self.core.main_window_id() == Some(id)
                } else {
                    self.popup == Some(id)
                };
                match esc_step(EscState {
                    shown,
                    note: self.shot.open(),
                    confirm: self.confirm_clear.is_some(),
                    menus: self.lang_open || self.corners_open || self.limit_open,
                    settings: self.settings_open,
                    has_query: !self.query.is_empty(),
                }) {
                    EscStep::Note => return self.finish_note(false),
                    EscStep::Confirm => self.confirm_clear = None,
                    EscStep::Menus => {
                        self.lang_open = false;
                        self.corners_open = false;
                        self.limit_open = false;
                    }
                    EscStep::Back => {
                        self.settings_open = false;
                        return text_input::focus(SEARCH_ID.clone());
                    }
                    EscStep::ClearSearch => {
                        self.query.clear();
                        self.ensure_selection();
                    }
                    EscStep::Close if self.window_mode => return self.update(Message::CloseRequested(id)),
                    EscStep::Close => return self.update(Message::TogglePopup),
                    EscStep::Nothing => {}
                }
            }
            Message::Rectangle(update) => match update {
                RectangleUpdate::Rectangle(r) => self.rectangle = r.1,
                RectangleUpdate::Init(tracker) => self.tracker = Some(tracker),
            },
            Message::CloseRequested(id) => {
                if self.window_mode {
                    return self.shutdown();
                }
                if self.popup == Some(id) {
                    self.popup = None;
                    self.popup_closed();
                    return self.shot_popup_closed();
                }
            }
            Message::TogglePopup => {
                if self.window_mode {
                    return Task::none();
                }
                if self.popup.is_some() {
                    return self.close_popup();
                }
                self.reload_store();
                let focus = if matches!(self.shot, shot::Phase::Note { .. }) {
                    text_input::focus(NOTE_ID.clone())
                } else {
                    text_input::focus(SEARCH_ID.clone())
                };
                return Task::batch([
                    cosmic::surface::surface_task(cosmic::surface::action::app_popup(
                        |_| Default::default(),
                        |app: &mut Self| {
                            let id = window::Id::unique();
                            app.popup = Some(id);
                            let mut settings = app.core.applet.get_popup_settings(
                                app.core.main_window_id().unwrap(),
                                id,
                                None,
                                None,
                                None,
                            );
                            let Rectangle { x, y, width, height } = app.rectangle;
                            settings.positioner.anchor_rect = Rectangle::<i32> {
                                x: x.max(1.0) as i32,
                                y: y.max(1.0) as i32,
                                width: width.max(1.0) as i32,
                                height: height.max(1.0) as i32,
                            };
                            settings.positioner.size = None;
                            settings
                        },
                        None,
                    )),
                    focus,
                ]);
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        look::keep_fonts();
        if self.window_mode {
            return corners::apply_window(&self.corners, self.popup_content());
        }
        let horizontal = matches!(self.core.applet.anchor, PanelAnchor::Top | PanelAnchor::Bottom);
        let padding = self.core.applet.suggested_padding(true).0;
        let size = self.core.applet.suggested_size(true).0 as f32;
        let icon_bytes = if self.prefs.paused { GLYPH_PAUSED } else { GLYPH_CLIPBOARD };
        let btn = button::custom(svg_glyph(icon_bytes, size, 1.0, false))
            .padding(if horizontal { [0, padding] } else { [padding, 0] })
            .on_press_down(Message::TogglePopup)
            .class(theme::Button::AppletIcon);
        let content: Element<Message> = match self.tracker.as_ref() {
            Some(tracker) => tracker.container(0, btn).ignore_bounds(true).into(),
            None => btn.into(),
        };
        autosize::autosize(content, AUTOSIZE_ID.clone()).into()
    }

    fn view_window(&self, _id: window::Id) -> Element<'_, Message> {
        look::keep_fonts();
        if self.window_mode {
            return corners::apply_window(&self.corners, self.popup_content());
        }
        corners::apply(&self.corners, self.core.applet.popup_container(self.popup_content()))
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        if self.window_mode {
            None
        } else {
            Some(cosmic::applet::style())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_closes_innermost_first() {
        let all = EscState {
            shown: true,
            note: true,
            confirm: true,
            menus: true,
            settings: true,
            has_query: true,
        };
        assert_eq!(esc_step(all), EscStep::Note);
        assert_eq!(esc_step(EscState { note: false, ..all }), EscStep::Confirm);
        assert_eq!(esc_step(EscState { note: false, confirm: false, ..all }), EscStep::Menus);
        assert_eq!(esc_step(EscState { note: false, confirm: false, menus: false, ..all }), EscStep::Back);
        assert_eq!(
            esc_step(EscState { note: false, confirm: false, menus: false, settings: false, ..all }),
            EscStep::ClearSearch
        );
        assert_eq!(esc_step(EscState { shown: true, ..Default::default() }), EscStep::Close);
        assert_eq!(esc_step(EscState::default()), EscStep::Nothing);
    }

    #[test]
    fn cli_mode_detection() {
        assert_eq!(decide_cli_mode(&[], false), CliAction::Window { delete_mode: false });
        assert_eq!(decide_cli_mode(&["-d".into()], false), CliAction::Window { delete_mode: true });
        assert_eq!(decide_cli_mode(&["--delete".into()], true), CliAction::Window { delete_mode: true });
        assert_eq!(decide_cli_mode(&["--window".into()], true), CliAction::Window { delete_mode: false });
        assert_eq!(decide_cli_mode(&[], true), CliAction::Applet);
        assert_eq!(decide_cli_mode(&["--applet".into()], false), CliAction::Applet);
        assert_eq!(decide_cli_mode(&["--version".into()], false), CliAction::Version);
        assert_eq!(decide_cli_mode(&["--help".into()], false), CliAction::Help);
    }
}
