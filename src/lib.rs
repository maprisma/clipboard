//! Maprisma Clipboard: fast, local Wayland clipboard manager for COSMIC (panel applet),
//! KDE Plasma and Hyprland (tray + standalone `--window`).
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const APP_NAME: &str = "Maprisma Clipboard";
pub mod skin;
pub const APP_ID: &str = "io.github.ubruckhaus.CosmicMaprismaClipboard";
pub const WINDOW_APP_ID: &str = "io.github.ubruckhaus.CosmicMaprismaClipboard.Window";
pub const ICON: &str = "io.github.ubruckhaus.CosmicMaprismaClipboard-symbolic";
pub const ICON_PAUSED: &str = "clipboard-paused-symbolic";
pub const WINDOW_BUS: &str = "io.github.ubruckhaus.CosmicMaprismaClipboard.Window";
pub const TRAY_BUS: &str = "io.github.ubruckhaus.CosmicMaprismaClipboard.Tray";

pub static CLOSE_WINDOW: AtomicBool = AtomicBool::new(false);

pub struct WindowControl;

#[zbus::interface(name = "io.github.ubruckhaus.CosmicMaprismaClipboard.Window")]
impl WindowControl {
    fn close(&self) {
        CLOSE_WINDOW.store(true, Ordering::Relaxed);
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(fallback))
}

pub fn config_home() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config")
}

pub fn data_home() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}

pub fn state_home() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state")
}

pub fn data_dir() -> PathBuf {
    data_home().join("maprisma-clipboard")
}

/// Atomic tmp + rename in the same directory; creates parent directories if missing.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    write_atomic_mode(path, data, None)
}

/// Atomic tmp + rename with optional Unix permissions (e.g. `Some(0o600)` for private clipboard data).
pub fn write_atomic_mode(path: &Path, data: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

    let dir = path.parent().ok_or_else(|| std::io::Error::other("no folder"))?;
    if !dir.exists() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        if mode.is_some() {
            builder.mode(0o700);
        }
        builder.create(dir)?;
    }

    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(
        ".{}.{}.{serial}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));

    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    if let Some(m) = mode {
        opts.mode(m);
    }
    let write_res = (|| -> std::io::Result<()> {
        let mut file = opts.open(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        if let Some(m) = mode {
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(m));
        }
        Ok(())
    })();
    if let Err(err) = write_res {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

pub const HISTORY_LIMITS: [usize; 5] = [50, 100, 250, 500, 1000];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppPrefs {
    pub max_items: usize,
    pub close_on_copy: bool,
    pub capture_images: bool,
    pub ignore_secrets: bool,
    pub paused: bool,
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            max_items: 250,
            close_on_copy: true,
            capture_images: true,
            ignore_secrets: true,
            paused: false,
        }
    }
}

pub fn load_prefs() -> AppPrefs {
    let max_items = i18n::read_pref("max_items")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .filter(|n| HISTORY_LIMITS.contains(n))
        .unwrap_or(250);
    let close_on_copy = i18n::read_pref("close_on_copy").and_then(|v| v.as_bool()).unwrap_or(true);
    let capture_images = i18n::read_pref("capture_images").and_then(|v| v.as_bool()).unwrap_or(true);
    let ignore_secrets = i18n::read_pref("ignore_secrets").and_then(|v| v.as_bool()).unwrap_or(true);
    let paused = i18n::read_pref("paused").and_then(|v| v.as_bool()).unwrap_or(false);
    AppPrefs {
        max_items,
        close_on_copy,
        capture_images,
        ignore_secrets,
        paused,
    }
}

pub mod clipboard;
pub mod i18n;
pub mod store;
