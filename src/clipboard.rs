//! Wayland clipboard capture (event-driven `wl-paste --watch` + `wl-clipboard-rs` inspection,
//! with automatic polling fallback) and clipboard writing (`wl-copy` + `wl-clipboard-rs` fallback).
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use wl_clipboard_rs::copy::{MimeType as CopyMime, Options as CopyOptions, Source};
use wl_clipboard_rs::paste::{self, ClipboardType, MimeType, Seat};

use crate::store::{ClipboardStore, now_unix};

const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 25 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOutcome {
    Captured,
    Unchanged,
    Paused,
    IgnoredSecret,
    Empty,
}

/// Checks whether the offered MIME types mark the clipboard payload as a password manager secret.
pub fn has_secret_mime(mimes: &[String]) -> bool {
    mimes.iter().any(|m| {
        let l = m.to_ascii_lowercase();
        l == "x-kde-passwordmanagerhint" || l == "application/x-keepassxc-secret"
    })
}

fn is_secret_clipboard(mimes: &[String]) -> bool {
    if mimes.iter().any(|m| m.eq_ignore_ascii_case("application/x-keepassxc-secret")) {
        return true;
    }
    if let Some(hint_mime) = mimes.iter().find(|m| m.eq_ignore_ascii_case("x-kde-passwordmanagerhint")) {
        if let Ok((mut pipe, _)) = paste::get_contents(
            ClipboardType::Regular,
            Seat::Unspecified,
            MimeType::Specific(hint_mime),
        ) {
            let mut buf = String::new();
            let _ = pipe.read_to_string(&mut buf);
            let val = buf.trim().to_ascii_lowercase();
            // KDE / KeePassXC / Bitwarden put "secret" here; treat empty or unknown safely as secret.
            return val != "public" && val != "false" && val != "0";
        }
        return true;
    }
    false
}

fn read_mime_capped(mime: &str, max_bytes: usize) -> Option<Vec<u8>> {
    let (pipe, _) = paste::get_contents(
        ClipboardType::Regular,
        Seat::Unspecified,
        MimeType::Specific(mime),
    )
    .ok()?;
    let mut buf = Vec::new();
    pipe.take((max_bytes + 1) as u64).read_to_end(&mut buf).ok()?;
    if buf.is_empty() || buf.len() > max_bytes {
        return None;
    }
    Some(buf)
}

/// Picks the best text MIME type if offered.
fn pick_text_mime(mimes: &[String]) -> Option<&str> {
    for preferred in ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING"] {
        if let Some(m) = mimes.iter().find(|m| m.eq_ignore_ascii_case(preferred)) {
            return Some(m.as_str());
        }
    }
    None
}

/// Picks the best image MIME type if offered.
fn pick_image_mime(mimes: &[String]) -> Option<&str> {
    for preferred in ["image/png", "image/webp", "image/jpeg"] {
        if let Some(m) = mimes.iter().find(|m| m.eq_ignore_ascii_case(preferred)) {
            return Some(m.as_str());
        }
    }
    None
}

/// Reads the current Wayland regular clipboard once and persists it into `dir` if new.
/// Tracks `last_hash` to avoid redundant disk writes when nothing changed.
pub fn capture_once(dir: &Path, last_hash: &mut Option<String>) -> CaptureOutcome {
    crate::i18n::reload_prefs();
    let prefs = crate::load_prefs();
    if prefs.paused {
        return CaptureOutcome::Paused;
    }
    let Ok(mime_set) = paste::get_mime_types(ClipboardType::Regular, Seat::Unspecified) else {
        return CaptureOutcome::Empty;
    };
    let mimes: Vec<String> = mime_set.into_iter().collect();
    if mimes.is_empty() {
        return CaptureOutcome::Empty;
    }
    if prefs.ignore_secrets && has_secret_mime(&mimes) && is_secret_clipboard(&mimes) {
        return CaptureOutcome::IgnoredSecret;
    }

    // Prefer plain text when present and non-empty; otherwise capture image if enabled.
    if let Some(mime) = pick_text_mime(&mimes)
        && let Some(bytes) = read_mime_capped(mime, MAX_TEXT_BYTES)
        && let Ok(text) = String::from_utf8(bytes)
        && !text.trim().is_empty()
    {
        let hash = crate::store::sha256_hex(text.as_bytes());
        if last_hash.as_deref() == Some(hash.as_str()) {
            return CaptureOutcome::Unchanged;
        }
        let mut store = ClipboardStore::load_from(dir);
        if store.add_text(text, now_unix(), prefs.max_items) && store.save_to(dir).is_ok() {
            *last_hash = Some(hash);
            return CaptureOutcome::Captured;
        }
        return CaptureOutcome::Unchanged;
    }

    if prefs.capture_images
        && let Some(mime) = pick_image_mime(&mimes)
        && let Some(bytes) = read_mime_capped(mime, MAX_IMAGE_BYTES)
    {
        let raw_hash = crate::store::sha256_hex(&bytes);
        if last_hash.as_deref() == Some(raw_hash.as_str()) {
            return CaptureOutcome::Unchanged;
        }
        let mut store = ClipboardStore::load_from(dir);
        if store
            .add_image(dir, &bytes, now_unix(), prefs.max_items)
            .unwrap_or(false)
            && store.save_to(dir).is_ok()
        {
            *last_hash = Some(raw_hash);
            return CaptureOutcome::Captured;
        }
    }

    CaptureOutcome::Empty
}

/// Copies UTF-8 text to the Wayland clipboard via `wl-copy` (which forks to keep serving after exit),
/// with `wl-clipboard-rs` fallback.
pub fn copy_text(text: &str) -> Result<(), String> {
    if try_wl_copy("text/plain;charset=utf-8", text.as_bytes()).is_ok() {
        return Ok(());
    }
    let opts = CopyOptions::new();
    opts.copy(Source::Bytes(text.as_bytes().to_vec().into()), CopyMime::Text)
        .map_err(|e| e.to_string())
}

/// Copies a PNG image file to the Wayland clipboard.
pub fn copy_image(path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if try_wl_copy("image/png", &bytes).is_ok() {
        return Ok(());
    }
    let opts = CopyOptions::new();
    opts.copy(Source::Bytes(bytes.into()), CopyMime::Specific("image/png".into()))
        .map_err(|e| e.to_string())
}

fn try_wl_copy(mime: &str, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut child = std::process::Command::new("wl-copy")
        .args(["--type", mime])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(data).map_err(|e| e.to_string())?;
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("wl-copy exited with {status}"))
    }
}

/// Spawns one `wl-paste --watch` helper that prints a line to stdout on every selection change.
fn spawn_wl_paste_trigger(
    extra_args: &[&str],
    tx: mpsc::UnboundedSender<()>,
) -> Option<tokio::task::JoinHandle<()>> {
    let mut cmd = tokio::process::Command::new("wl-paste");
    cmd.args(extra_args)
        .args(["--watch", "echo", "tick"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd.spawn().ok()?;
    let stdout = child.stdout.take()?;
    Some(tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(_)) = lines.next_line().await {
            if tx.send(()).is_err() {
                break;
            }
        }
        let _ = child.kill().await;
    }))
}

/// Runs the continuous Wayland clipboard watcher. Sends `()` on `on_change` whenever a new clip is stored.
pub async fn run_watcher(dir: PathBuf, on_change: mpsc::UnboundedSender<()>) {
    // Ensure first-run migration happens before watcher starts appending.
    let mig_dir = dir.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let prefs = crate::load_prefs();
        let _ = ClipboardStore::load_or_migrate(&mig_dir, prefs.max_items);
    })
    .await;

    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
    let text_watch = spawn_wl_paste_trigger(&["--no-newline"], ev_tx.clone());
    let img_watch = spawn_wl_paste_trigger(&["--type", "image/png"], ev_tx.clone());
    let event_driven = text_watch.is_some() || img_watch.is_some();

    let mut last_hash: Option<String> = None;

    if event_driven {
        // Initial capture once at startup
        let _ = ev_tx.send(());
        while ev_rx.recv().await.is_some() {
            // Coalesce burst events (e.g. text + image watchers firing together)
            tokio::time::sleep(Duration::from_millis(80)).await;
            while ev_rx.try_recv().is_ok() {}

            let d = dir.clone();
            let prev = last_hash.clone();
            let (outcome, next_hash) = tokio::task::spawn_blocking(move || {
                let mut h = prev;
                let out = capture_once(&d, &mut h);
                (out, h)
            })
            .await
            .unwrap_or((CaptureOutcome::Empty, last_hash));
            last_hash = next_hash;
            if outcome == CaptureOutcome::Captured {
                let _ = on_change.send(());
            }
        }
    }

    // Fallback (or if wl-paste exited): lightweight 500ms polling via wl-clipboard-rs
    loop {
        let d = dir.clone();
        let prev = last_hash.clone();
        let (outcome, next_hash) = tokio::task::spawn_blocking(move || {
            let mut h = prev;
            let out = capture_once(&d, &mut h);
            (out, h)
        })
        .await
        .unwrap_or((CaptureOutcome::Empty, last_hash));
        last_hash = next_hash;
        if outcome == CaptureOutcome::Captured {
            let _ = on_change.send(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_password_manager_secret_mimes() {
        assert!(has_secret_mime(&["text/plain".into(), "x-kde-passwordManagerHint".into()]));
        assert!(has_secret_mime(&["application/x-keepassxc-secret".into()]));
        assert!(!has_secret_mime(&["text/plain;charset=utf-8".into(), "image/png".into()]));
    }

    #[test]
    fn picks_preferred_mimes() {
        let mimes = vec!["text/html".into(), "text/plain".into(), "text/plain;charset=utf-8".into()];
        assert_eq!(pick_text_mime(&mimes), Some("text/plain;charset=utf-8"));
        let imgs = vec!["image/jpeg".into(), "image/png".into()];
        assert_eq!(pick_image_mime(&imgs), Some("image/png"));
    }
}
