//! KDE / Hyprland StatusNotifierItem bar icon + background Wayland clipboard watcher.
mod model;
mod present;

use std::{collections::HashMap, sync::Arc, time::Duration};

use maprisma_clipboard::clipboard;
use maprisma_clipboard::i18n::{self, choices};
use maprisma_clipboard::store::{ClipKind, ClipboardStore};
use maprisma_clipboard::{APP_NAME, ICON, ICON_PAUSED, TRAY_BUS, WINDOW_BUS, data_dir, load_prefs};
use model::Action;
use tokio::sync::{Mutex, mpsc};
use zbus::{
    Connection,
    object_server::SignalEmitter,
    zvariant::{OwnedObjectPath, OwnedValue, Value as Variant},
};

type Props = HashMap<String, OwnedValue>;
type Layout = (i32, Props, Vec<OwnedValue>);
type Pixmaps = Vec<(i32, i32, Vec<u8>)>;

struct State {
    paused: bool,
    count: usize,
    tooltip: String,
    menu: model::Item,
    revision: u32,
}

impl State {
    fn status(&self) -> &'static str {
        "Active"
    }

    fn icon_name(&self) -> &'static str {
        if self.paused { ICON_PAUSED } else { ICON }
    }

    fn tooltip(&self) -> String {
        self.tooltip.clone()
    }
}

type Shared = Arc<Mutex<State>>;

#[derive(Clone)]
struct Menu {
    state: Shared,
    tx: mpsc::UnboundedSender<Action>,
}

fn owned<T: Into<Variant<'static>>>(value: T) -> OwnedValue {
    OwnedValue::try_from(value.into()).unwrap()
}

fn properties(item: &model::Item, names: &[String]) -> Props {
    let mut props = Props::new();
    props.insert("label".into(), owned(item.label.clone()));
    props.insert("enabled".into(), owned(item.enabled));
    props.insert("visible".into(), owned(true));
    if !item.children.is_empty() {
        props.insert("children-display".into(), owned("submenu"));
    }
    if item.separator {
        props.insert("type".into(), owned("separator"));
    }
    if !names.is_empty() {
        props.retain(|key, _| names.contains(key));
    }
    props
}

fn layout(item: &model::Item, depth: i32, names: &[String]) -> Layout {
    (
        item.id,
        properties(item, names),
        if depth == 0 {
            vec![]
        } else {
            item.children
                .iter()
                .map(|c| {
                    OwnedValue::try_from(Variant::new(layout(
                        c,
                        if depth < 0 { -1 } else { depth - 1 },
                        names,
                    )))
                    .unwrap()
                })
                .collect()
        },
    )
}

impl Menu {
    async fn dispatch(&self, id: i32, event: &str) -> bool {
        let s = self.state.lock().await;
        let menu = s.menu.clone();
        let Some(item) = menu.find(id) else {
            return false;
        };
        if event == "clicked"
            && let Some(action) = menu.activate(item.id)
        {
            let _ = self.tx.send(action);
        }
        true
    }
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        vec![]
    }
    async fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Layout)> {
        let s = self.state.lock().await;
        let menu = s.menu.clone();
        let item = menu
            .find(parent_id)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("Unknown menu ID".into()))?;
        Ok((s.revision, layout(item, recursion_depth, &property_names)))
    }
    async fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, Props)> {
        let s = self.state.lock().await;
        let menu = s.menu.clone();
        ids.iter()
            .filter_map(|id| menu.find(*id).map(|i| (*id, properties(i, &property_names))))
            .collect()
    }
    async fn get_property(&self, id: i32, name: String) -> zbus::fdo::Result<OwnedValue> {
        let s = self.state.lock().await;
        let menu = s.menu.clone();
        menu.find(id)
            .and_then(|i| properties(i, &[]).remove(&name))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("Unknown menu property".into()))
    }
    async fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        self.dispatch(id, &event_id).await;
    }
    async fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        let mut errors = vec![];
        for (id, event, _, _) in events {
            if !self.dispatch(id, &event).await {
                errors.push(id);
            }
        }
        errors
    }
    async fn about_to_show(&self, _id: i32) -> bool {
        false
    }
    async fn about_to_show_group(&self, ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        let s = self.state.lock().await;
        let menu = s.menu.clone();
        (
            vec![],
            ids.into_iter().filter(|id| menu.find(*id).is_none()).collect(),
        )
    }
    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

struct Tray {
    state: Shared,
    tx: mpsc::UnboundedSender<Action>,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Tray {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }
    #[zbus(property)]
    fn id(&self) -> &str {
        "maprisma-clipboard"
    }
    #[zbus(property)]
    fn title(&self) -> &str {
        APP_NAME
    }
    #[zbus(property)]
    async fn status(&self) -> &str {
        self.state.lock().await.status()
    }
    #[zbus(property)]
    fn window_id(&self) -> u32 {
        0
    }
    #[zbus(property)]
    async fn icon_name(&self) -> &str {
        self.state.lock().await.icon_name()
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Pixmaps {
        vec![]
    }
    #[zbus(property)]
    fn overlay_icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> Pixmaps {
        vec![]
    }
    #[zbus(property)]
    async fn attention_icon_name(&self) -> &str {
        self.state.lock().await.icon_name()
    }
    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> Pixmaps {
        vec![]
    }
    #[zbus(property)]
    fn attention_movie_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    async fn tool_tip(&self) -> (String, Pixmaps, String, String) {
        let s = self.state.lock().await;
        (s.icon_name().into(), vec![], APP_NAME.into(), s.tooltip())
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from("/MenuBar").unwrap()
    }
    fn activate(&self, _x: i32, _y: i32) {
        let _ = self.tx.send(Action::Activate);
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {
        let _ = self.tx.send(Action::Activate);
    }
    fn context_menu(&self, _x: i32, _y: i32) {}
    fn scroll(&self, _delta: i32, _orientation: String) {}
    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

async fn window_owner(dbus: &zbus::fdo::DBusProxy<'_>) -> Option<zbus::names::OwnedUniqueName> {
    for _ in 0..40 {
        if let Ok(owner) = dbus.get_name_owner(WINDOW_BUS.try_into().ok()?).await {
            return Some(owner);
        }
        if !STARTED.load(std::sync::atomic::Ordering::Relaxed) {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    None
}

async fn window_action(
    connection: &Connection,
    toggle: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let dbus = zbus::fdo::DBusProxy::new(connection).await?;
    if let Some(owner) = window_owner(&dbus).await {
        if toggle {
            let proxy = zbus::Proxy::new(connection, owner.as_str(), "/Window", WINDOW_BUS).await?;
            proxy.call::<_, _, ()>("Close", &()).await?;
        } else {
            present::present(connection, owner.as_str()).await;
        }
    } else {
        let executable = std::env::current_exe()?.with_file_name("maprisma-clipboard");
        let mut child = tokio::process::Command::new(executable)
            .arg("--window")
            .stdin(std::process::Stdio::null())
            .spawn()?;
        STARTED.store(true, std::sync::atomic::Ordering::Relaxed);
        tokio::spawn(async move {
            let _ = child.wait().await;
            STARTED.store(false, std::sync::atomic::Ordering::Relaxed);
        });
    }
    Ok(())
}

async fn refresh_state(
    connection: &Connection,
    state: &Shared,
    dir: &std::path::Path,
    lang_error: bool,
) {
    let d = dir.to_path_buf();
    let (store, paused) = tokio::task::spawn_blocking(move || {
        i18n::reload_prefs();
        let prefs = load_prefs();
        let store = ClipboardStore::load_from(&d);
        (store, prefs.paused)
    })
    .await
    .unwrap_or_default();

    let count = store.items.len();
    let new_tooltip = model::tooltip(count, paused);
    let new_menu = model::menu(&store.items, paused, lang_error);

    let (icon_changed, tip_changed, menu_changed, rev) = {
        let mut s = state.lock().await;
        let icon_changed = s.paused != paused;
        let tip_changed = s.tooltip != new_tooltip || s.count != count;
        let menu_changed = s.menu != new_menu;
        s.paused = paused;
        s.count = count;
        s.tooltip = new_tooltip;
        if menu_changed {
            s.menu = new_menu;
            s.revision = s.revision.wrapping_add(1);
        }
        (icon_changed, tip_changed, menu_changed, s.revision)
    };

    if let Ok(emitter) = SignalEmitter::new(connection, "/StatusNotifierItem") {
        if icon_changed {
            let _ = Tray::new_icon(&emitter).await;
        }
        if tip_changed || icon_changed {
            let _ = Tray::new_tool_tip(&emitter).await;
        }
    }
    if menu_changed
        && let Ok(emitter) = SignalEmitter::new(connection, "/MenuBar")
    {
        let _ = Menu::layout_updated(&emitter, rev, 0).await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = data_dir();
    let prefs = load_prefs();
    let store = ClipboardStore::load_or_migrate(&dir, prefs.max_items);

    let initial = State {
        paused: prefs.paused,
        count: store.items.len(),
        tooltip: model::tooltip(store.items.len(), prefs.paused),
        menu: model::menu(&store.items, prefs.paused, false),
        revision: 1,
    };
    let state = Arc::new(Mutex::new(initial));
    let (tx, mut rx) = mpsc::unbounded_channel();

    let connection = zbus::connection::Builder::session()?
        .serve_at(
            "/StatusNotifierItem",
            Tray {
                state: state.clone(),
                tx: tx.clone(),
            },
        )?
        .serve_at(
            "/MenuBar",
            Menu {
                state: state.clone(),
                tx: tx.clone(),
            },
        )?
        .build()
        .await?;

    // Single tray instance per session.
    match connection
        .request_name_with_flags(TRAY_BUS, zbus::fdo::RequestNameFlags::DoNotQueue.into())
        .await
    {
        Ok(_) => {}
        Err(zbus::Error::NameTaken) => return Ok(()),
        Err(error) => return Err(error.into()),
    }

    // Register with StatusNotifierWatcher (handles Plasma/Waybar/Maprisma Shell restarts too).
    let watcher_connection = connection.clone();
    tokio::spawn(async move {
        let mut registered = String::new();
        loop {
            let dbus = zbus::fdo::DBusProxy::new(&watcher_connection).await;
            let owner = match dbus {
                Ok(p) => p
                    .get_name_owner("org.kde.StatusNotifierWatcher".try_into().unwrap())
                    .await
                    .ok()
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                Err(_) => String::new(),
            };
            if owner.is_empty() {
                registered.clear();
            } else if owner != registered {
                if let Ok(proxy) = zbus::Proxy::new(
                    &watcher_connection,
                    owner.as_str(),
                    "/StatusNotifierWatcher",
                    "org.kde.StatusNotifierWatcher",
                )
                .await
                    && proxy
                        .call::<_, _, ()>(
                            "RegisterStatusNotifierItem",
                            &(watcher_connection.unique_name().unwrap().as_str(),),
                        )
                        .await
                        .is_ok()
                {
                    registered = owner;
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    // Continuous Wayland clipboard watcher
    let (clip_tx, mut clip_rx) = mpsc::unbounded_channel();
    tokio::spawn(clipboard::run_watcher(dir.clone(), clip_tx));

    // Watch external edits to history.json / config.json (e.g. from the UI window)
    let mut poll_tick = tokio::time::interval(Duration::from_secs(1));
    let mut last_hist_mtime = ClipboardStore::history_mtime(&dir);
    let mut last_cfg_mtime = std::fs::metadata(i18n::config_path()).ok().and_then(|m| m.modified().ok());

    loop {
        tokio::select! {
            Some(()) = clip_rx.recv() => {
                while clip_rx.try_recv().is_ok() {}
                last_hist_mtime = ClipboardStore::history_mtime(&dir);
                refresh_state(&connection, &state, &dir, false).await;
            }
            _ = poll_tick.tick() => {
                let hist_m = ClipboardStore::history_mtime(&dir);
                let cfg_m = std::fs::metadata(i18n::config_path()).ok().and_then(|m| m.modified().ok());
                if hist_m != last_hist_mtime || cfg_m != last_cfg_mtime {
                    last_hist_mtime = hist_m;
                    last_cfg_mtime = cfg_m;
                    refresh_state(&connection, &state, &dir, false).await;
                }
            }
            Some(action) = rx.recv() => {
                match action {
                    Action::Quit => return Ok(()),
                    Action::Open | Action::Activate => {
                        if let Err(error) = window_action(&connection, action == Action::Activate).await {
                            eprintln!("Window: {error}");
                        }
                    }
                    Action::TogglePause => {
                        let next = !load_prefs().paused;
                        let _ = i18n::write_pref("paused", Some(next.into()));
                        refresh_state(&connection, &state, &dir, false).await;
                    }
                    Action::CopyClip(id) => {
                        let d = dir.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            let store = ClipboardStore::load_from(&d);
                            if let Some(entry) = store.items.iter().find(|e| e.id == id) {
                                match &entry.kind {
                                    ClipKind::Image { .. } => {
                                        if let Some(path) = entry.image_path(&d) {
                                            let _ = clipboard::copy_image(&path);
                                        }
                                    }
                                    _ => {
                                        let _ = clipboard::copy_text(&entry.text);
                                    }
                                }
                            }
                        }).await;
                    }
                    Action::ClearUnpinned => {
                        let d = dir.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            let mut store = ClipboardStore::load_from(&d);
                            store.clear_unpinned(Some(&d));
                            let _ = store.save_to(&d);
                        }).await;
                        refresh_state(&connection, &state, &dir, false).await;
                    }
                    Action::Language(idx) => {
                        let list = choices();
                        let mut err = false;
                        if let Some((code, _)) = list.get(idx) {
                            err = i18n::save(code).is_err();
                        }
                        refresh_state(&connection, &state, &dir, err).await;
                    }
                }
            }
        }
    }
}
