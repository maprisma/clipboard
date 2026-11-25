//! Persistent local clipboard store (`~/.local/share/maprisma-clipboard/history.json` + `images/<hash>.png`),
//! content classification, color parsing, deduplication, pinning, pruning, search/filtering, and cliphist migration.
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorRgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ColorRgba {
    pub fn hex(&self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }

    pub fn rgb_string(&self) -> String {
        if self.a == 255 {
            format!("rgb({}, {}, {})", self.r, self.g, self.b)
        } else {
            let a = (self.a as f32 / 255.0 * 100.0).round() / 100.0;
            format!("rgba({}, {}, {}, {a})", self.r, self.g, self.b)
        }
    }

    pub fn hsl_string(&self) -> String {
        let r = self.r as f32 / 255.0;
        let g = self.g as f32 / 255.0;
        let b = self.b as f32 / 255.0;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let l = (max + min) / 2.0;
        let d = max - min;
        let (h, s) = if d.abs() < 1e-6 {
            (0.0, 0.0)
        } else {
            let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
            let mut h = if (max - r).abs() < 1e-6 {
                (g - b) / d + if g < b { 6.0 } else { 0.0 }
            } else if (max - g).abs() < 1e-6 {
                (b - r) / d + 2.0
            } else {
                (r - g) / d + 4.0
            };
            h *= 60.0;
            (h, s)
        };
        format!("hsl({}°, {}%, {}%)", h.round() as i32, (s * 100.0).round() as i32, (l * 100.0).round() as i32)
    }

    pub fn is_light(&self) -> bool {
        let luma = 0.2126 * (self.r as f32 / 255.0)
            + 0.7152 * (self.g as f32 / 255.0)
            + 0.0722 * (self.b as f32 / 255.0);
        luma > 0.55
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClipKind {
    Text,
    Code,
    Link,
    Color(ColorRgba),
    Email,
    Image {
        width: u32,
        height: u32,
        bytes: usize,
        file: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipEntry {
    pub id: String,
    pub kind: ClipKind,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub pinned: bool,
    pub timestamp: u64,
    #[serde(default)]
    pub chars: usize,
    #[serde(default)]
    pub lines: usize,
}

impl ClipEntry {
    pub fn preview_line(&self, max_chars: usize) -> String {
        if let ClipKind::Image { width, height, .. } = &self.kind {
            return crate::i18n::trf("Image · {0}×{1}", &[width.to_string(), height.to_string()]);
        }
        let mut out = String::new();
        let mut prev_space = true;
        for ch in self.text.chars() {
            if ch.is_whitespace() {
                if !prev_space {
                    out.push(' ');
                    prev_space = true;
                }
            } else {
                out.push(ch);
                prev_space = false;
            }
            if out.chars().count() >= max_chars {
                break;
            }
        }
        let trimmed = out.trim_end();
        if self.text.trim().chars().count() > max_chars && trimmed.chars().count() >= max_chars {
            let mut s: String = trimmed.chars().take(max_chars.saturating_sub(1)).collect();
            s.push('…');
            s
        } else {
            trimmed.to_string()
        }
    }

    pub fn url_domain(&self) -> Option<String> {
        if !matches!(self.kind, ClipKind::Link) {
            return None;
        }
        let rest = self
            .text
            .trim()
            .strip_prefix("https://")
            .or_else(|| self.text.trim().strip_prefix("http://"))?;
        let host = rest.split(['/', '?', '#']).next()?;
        let host = host.rsplit('@').next().unwrap_or(host);
        let host = host.split(':').next().unwrap_or(host);
        let host = host.strip_prefix("www.").unwrap_or(host);
        (!host.is_empty()).then(|| host.to_string())
    }

    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        let mut prev_space = true;
        for ch in self.text.chars() {
            if ch.is_whitespace() {
                if !prev_space {
                    out.push(' ');
                    prev_space = true;
                }
            } else {
                out.push(ch);
                prev_space = false;
            }
        }
        out.trim().to_string()
    }

    pub fn image_path(&self, dir: &Path) -> Option<PathBuf> {
        match &self.kind {
            ClipKind::Image { file, .. } => {
                // Prevent path traversal: only plain file names under images/
                let name = Path::new(file).file_name()?;
                Some(dir.join("images").join(name))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CategoryFilter {
    #[default]
    All,
    Pinned,
    Text,
    Images,
    Links,
    Colors,
}

impl CategoryFilter {
    pub const ALL: [CategoryFilter; 6] = [
        CategoryFilter::All,
        CategoryFilter::Pinned,
        CategoryFilter::Text,
        CategoryFilter::Images,
        CategoryFilter::Links,
        CategoryFilter::Colors,
    ];

    pub fn label_key(self) -> &'static str {
        match self {
            CategoryFilter::All => "All",
            CategoryFilter::Pinned => "Pinned",
            CategoryFilter::Text => "Text",
            CategoryFilter::Images => "Images",
            CategoryFilter::Links => "Links",
            CategoryFilter::Colors => "Colors",
        }
    }

    pub fn matches(self, entry: &ClipEntry) -> bool {
        match self {
            CategoryFilter::All => true,
            CategoryFilter::Pinned => entry.pinned,
            CategoryFilter::Text => matches!(entry.kind, ClipKind::Text | ClipKind::Code | ClipKind::Email),
            CategoryFilter::Images => matches!(entry.kind, ClipKind::Image { .. }),
            CategoryFilter::Links => matches!(entry.kind, ClipKind::Link),
            CategoryFilter::Colors => matches!(entry.kind, ClipKind::Color(_)),
        }
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

/// Parses CSS color strings: `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`, `rgb(...)`, `rgba(...)`, `hsl(...)`, `hsla(...)`.
pub fn parse_color(input: &str) -> Option<ColorRgba> {
    let s = input.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        return match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                Some(ColorRgba { r, g, b, a: 255 })
            }
            4 => {
                let r = u8::from_str_radix(&hex[0..1], 16).ok()? * 17;
                let g = u8::from_str_radix(&hex[1..2], 16).ok()? * 17;
                let b = u8::from_str_radix(&hex[2..3], 16).ok()? * 17;
                let a = u8::from_str_radix(&hex[3..4], 16).ok()? * 17;
                Some(ColorRgba { r, g, b, a })
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(ColorRgba { r, g, b, a: 255 })
            }
            8 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
                Some(ColorRgba { r, g, b, a })
            }
            _ => None,
        };
    }

    let lower = s.to_ascii_lowercase();
    if (lower.starts_with("rgb(") || lower.starts_with("rgba(")) && lower.ends_with(')') {
        let inner = &lower[lower.find('(')? + 1..lower.len() - 1];
        let parts = split_css_args(inner);
        if parts.len() != 3 && parts.len() != 4 {
            return None;
        }
        let r = parse_rgb_channel(parts[0])?;
        let g = parse_rgb_channel(parts[1])?;
        let b = parse_rgb_channel(parts[2])?;
        let a = if parts.len() == 4 { parse_alpha(parts[3])? } else { 255 };
        return Some(ColorRgba { r, g, b, a });
    }

    if (lower.starts_with("hsl(") || lower.starts_with("hsla(")) && lower.ends_with(')') {
        let inner = &lower[lower.find('(')? + 1..lower.len() - 1];
        let parts = split_css_args(inner);
        if parts.len() != 3 && parts.len() != 4 {
            return None;
        }
        let h_str = parts[0].strip_suffix("deg").or_else(|| parts[0].strip_suffix('°')).unwrap_or(parts[0]);
        let h = h_str.parse::<f32>().ok()?;
        let s = parts[1].strip_suffix('%')?.parse::<f32>().ok()?;
        let l = parts[2].strip_suffix('%')?.parse::<f32>().ok()?;
        if !h.is_finite() || !(0.0..=100.0).contains(&s) || !(0.0..=100.0).contains(&l) {
            return None;
        }
        let a = if parts.len() == 4 { parse_alpha(parts[3])? } else { 255 };
        let (r, g, b) = hsl_to_rgb(h.rem_euclid(360.0), s / 100.0, l / 100.0);
        return Some(ColorRgba { r, g, b, a });
    }

    None
}

fn split_css_args(inner: &str) -> Vec<&str> {
    inner
        .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect()
}

fn parse_rgb_channel(s: &str) -> Option<u8> {
    if let Some(pct) = s.strip_suffix('%') {
        let v = pct.parse::<f32>().ok()?;
        if !(0.0..=100.0).contains(&v) {
            return None;
        }
        Some((v * 2.55).round() as u8)
    } else {
        let v = s.parse::<f32>().ok()?;
        if !(0.0..=255.0).contains(&v) {
            return None;
        }
        Some(v.round() as u8)
    }
}

fn parse_alpha(s: &str) -> Option<u8> {
    if let Some(pct) = s.strip_suffix('%') {
        let v = pct.parse::<f32>().ok()?;
        if !(0.0..=100.0).contains(&v) {
            return None;
        }
        Some((v * 2.55).round() as u8)
    } else {
        let v = s.parse::<f32>().ok()?;
        if !(0.0..=1.0).contains(&v) {
            return None;
        }
        Some((v * 255.0).round() as u8)
    }
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = match h {
        h if h < 60.0 => (c, x, 0.0),
        h if h < 120.0 => (x, c, 0.0),
        h if h < 180.0 => (0.0, c, x),
        h if h < 240.0 => (0.0, x, c),
        h if h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b1 + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

pub fn classify_text(text: &str) -> ClipKind {
    let trimmed = text.trim();
    if let Some(color) = parse_color(trimmed) {
        return ClipKind::Color(color);
    }
    if is_single_url(trimmed) {
        return ClipKind::Link;
    }
    if is_email(trimmed) {
        return ClipKind::Email;
    }
    if looks_like_code(trimmed) {
        return ClipKind::Code;
    }
    ClipKind::Text
}

fn is_single_url(s: &str) -> bool {
    if s.is_empty() || s.chars().any(char::is_whitespace) {
        return false;
    }
    let Some(rest) = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")) else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    !host.is_empty() && (host.contains('.') || host.eq_ignore_ascii_case("localhost"))
}

fn is_email(s: &str) -> bool {
    if s.is_empty() || s.len() > 254 || s.chars().any(char::is_whitespace) {
        return false;
    }
    let mut parts = s.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return false;
    }
    if domain.starts_with('.') || domain.ends_with('.') || domain.contains("..") {
        return false;
    }
    local.chars().all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c))
        && domain.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
}

fn looks_like_code(s: &str) -> bool {
    if s.starts_with("#!/") {
        return true;
    }
    // Valid JSON object or array
    if ((s.starts_with('{') && s.ends_with('}')) || (s.starts_with('[') && s.ends_with(']')))
        && serde_json::from_str::<serde_json::Value>(s).is_ok()
    {
        return true;
    }
    // Single-line shell commands
    let cmd_prefixes = [
        "cargo ", "git ", "sudo ", "docker ", "systemctl ", "kubectl ", "curl ", "ssh ", "scp ",
        "rsync ", "hyprctl ", "pacman ", "paru ", "yay ", "npm ", "pnpm ", "python3 ", "pytest ",
        "gh ", "wl-copy ", "wl-paste ",
    ];
    if cmd_prefixes.iter().any(|p| s.starts_with(p)) {
        return true;
    }
    // Explicit code syntax markers
    let strong_markers = [
        "fn ", "pub fn ", "impl ", "struct ", "enum ", "use std::", "def ", "async fn ",
        "function ", "export default", "const ", "import {", "from ", "SELECT ", "CREATE TABLE",
        "<!DOCTYPE", "<html", "<svg", "<?php", "#include ",
    ];
    if strong_markers.iter().any(|m| s.contains(m)) {
        return true;
    }
    // Multi-line syntax density
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() >= 2 {
        let mut score = 0;
        if s.contains('{') && s.contains('}') {
            score += 2;
        }
        if s.contains("->") || s.contains("=>") || s.contains("::") || s.contains("&&") || s.contains("||") {
            score += 2;
        }
        if lines.iter().filter(|l| l.starts_with("  ") || l.starts_with('\t')).count() >= 1 {
            score += 1;
        }
        if lines.iter().filter(|l| l.trim_end().ends_with(';')).count() >= 1 {
            score += 1;
        }
        if score >= 3 {
            return true;
        }
    }
    false
}

pub fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn relative_time(timestamp: u64, now: u64) -> String {
    let diff = now.saturating_sub(timestamp);
    if diff < 10 {
        crate::i18n::tr("just now")
    } else if diff < 60 {
        crate::i18n::trf("{0}s ago", &[diff.to_string()])
    } else if diff < 3600 {
        crate::i18n::trf("{0}m ago", &[(diff / 60).to_string()])
    } else if diff < 86_400 {
        crate::i18n::trf("{0}h ago", &[(diff / 3600).to_string()])
    } else {
        crate::i18n::trf("{0}d ago", &[(diff / 86_400).to_string()])
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ClipboardStore {
    #[serde(default)]
    pub items: Vec<ClipEntry>,
}

impl ClipboardStore {
    pub fn history_path(dir: &Path) -> PathBuf {
        dir.join("history.json")
    }

    pub fn history_mtime(dir: &Path) -> Option<SystemTime> {
        std::fs::metadata(Self::history_path(dir)).ok()?.modified().ok()
    }

    pub fn load_from(dir: &Path) -> Self {
        let path = Self::history_path(dir);
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        let mut store: Self = serde_json::from_slice(&bytes).unwrap_or_default();
        store.sort_entries();
        store
    }

    /// Loads `history.json`, or on very first run imports up to 50 recent `cliphist` entries if available.
    pub fn load_or_migrate(dir: &Path, max_items: usize) -> Self {
        let path = Self::history_path(dir);
        if path.exists() {
            return Self::load_from(dir);
        }
        let mut store = Self::default();
        if import_from_cliphist(&mut store, dir, max_items, 50) > 0 {
            let _ = store.save_to(dir);
        }
        store
    }

    pub fn save_to(&self, dir: &Path) -> io::Result<()> {
        let data = serde_json::to_string_pretty(self)? + "\n";
        crate::write_atomic_mode(&Self::history_path(dir), data.as_bytes(), Some(0o600))
    }

    pub fn sort_entries(&mut self) {
        self.items.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| b.timestamp.cmp(&a.timestamp))
                .then_with(|| a.id.cmp(&b.id))
        });
    }

    pub fn add_text(&mut self, text: String, now: u64, max_items: usize) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        let id = sha256_hex(text.as_bytes());
        if let Some(pos) = self.items.iter().position(|e| e.id == id) {
            let mut entry = self.items.remove(pos);
            entry.timestamp = now;
            self.items.push(entry);
            self.sort_entries();
            self.prune(max_items, None);
            return true;
        }
        let kind = classify_text(&text);
        let chars = text.chars().count();
        let lines = text.lines().count().max(1);
        self.items.push(ClipEntry {
            id,
            kind,
            text,
            pinned: false,
            timestamp: now,
            chars,
            lines,
        });
        self.sort_entries();
        self.prune(max_items, None);
        true
    }

    pub fn add_image(&mut self, dir: &Path, raw_bytes: &[u8], now: u64, max_items: usize) -> io::Result<bool> {
        if raw_bytes.is_empty() {
            return Ok(false);
        }
        let img = image::load_from_memory(raw_bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let (width, height) = (img.width(), img.height());
        if width == 0 || height == 0 {
            return Ok(false);
        }
        // Keep original PNG if it already starts with PNG signature; otherwise normalize to PNG.
        let png_bytes = if raw_bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            raw_bytes.to_vec()
        } else {
            let mut buf = Vec::new();
            img.write_to(&mut io::Cursor::new(&mut buf), image::ImageFormat::Png)
                .map_err(|e| io::Error::other(e.to_string()))?;
            buf
        };

        let id = sha256_hex(&png_bytes);
        let file = format!("{id}.png");
        let img_path = dir.join("images").join(&file);
        if !img_path.exists() {
            crate::write_atomic_mode(&img_path, &png_bytes, Some(0o600))?;
        }

        if let Some(pos) = self.items.iter().position(|e| e.id == id) {
            let mut entry = self.items.remove(pos);
            entry.timestamp = now;
            self.items.push(entry);
            self.sort_entries();
            self.prune(max_items, Some(dir));
            return Ok(true);
        }

        self.items.push(ClipEntry {
            id,
            kind: ClipKind::Image {
                width,
                height,
                bytes: png_bytes.len(),
                file,
            },
            text: String::new(),
            pinned: false,
            timestamp: now,
            chars: 0,
            lines: 0,
        });
        self.sort_entries();
        self.prune(max_items, Some(dir));
        Ok(true)
    }

    pub fn toggle_pin(&mut self, id: &str) -> bool {
        if let Some(entry) = self.items.iter_mut().find(|e| e.id == id) {
            entry.pinned = !entry.pinned;
            self.sort_entries();
            return true;
        }
        false
    }

    pub fn remove(&mut self, id: &str) -> Option<ClipEntry> {
        let pos = self.items.iter().position(|e| e.id == id)?;
        Some(self.items.remove(pos))
    }

    pub fn restore(&mut self, entry: ClipEntry) {
        if !self.items.iter().any(|e| e.id == entry.id) {
            self.items.push(entry);
            self.sort_entries();
        }
    }

    pub fn clear_unpinned(&mut self, dir: Option<&Path>) {
        self.items.retain(|e| e.pinned);
        if let Some(dir) = dir {
            self.cleanup_orphan_images(dir);
        }
    }

    pub fn clear_all(&mut self, dir: Option<&Path>) {
        self.items.clear();
        if let Some(dir) = dir {
            self.cleanup_orphan_images(dir);
        }
    }

    /// Prunes excess unpinned items beyond `max_items`; pinned items are never auto-pruned.
    pub fn prune(&mut self, max_items: usize, dir: Option<&Path>) {
        let mut unpinned_seen = 0usize;
        self.items.retain(|e| {
            if e.pinned {
                true
            } else {
                unpinned_seen += 1;
                unpinned_seen <= max_items
            }
        });
        if let Some(dir) = dir {
            self.cleanup_orphan_images(dir);
        }
    }

    pub fn cleanup_orphan_images(&self, dir: &Path) {
        let images_dir = dir.join("images");
        let Ok(read_dir) = std::fs::read_dir(&images_dir) else {
            return;
        };
        let keep: HashSet<&str> = self
            .items
            .iter()
            .filter_map(|e| match &e.kind {
                ClipKind::Image { file, .. } => Some(file.as_str()),
                _ => None,
            })
            .collect();
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".png") && !keep.contains(name_str.as_ref()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    pub fn filtered(&self, query: &str, filter: CategoryFilter) -> Vec<&ClipEntry> {
        let q = query.trim().to_ascii_lowercase();
        self.items
            .iter()
            .filter(|e| filter.matches(e))
            .filter(|e| {
                if q.is_empty() {
                    return true;
                }
                if e.text.to_ascii_lowercase().contains(&q) {
                    return true;
                }
                match &e.kind {
                    ClipKind::Color(c) => {
                        c.hex().to_ascii_lowercase().contains(&q)
                            || c.rgb_string().to_ascii_lowercase().contains(&q)
                            || "color".contains(&q)
                    }
                    ClipKind::Image { width, height, .. } => {
                        format!("image {width}x{height} {width}×{height}").contains(&q)
                    }
                    ClipKind::Link => e
                        .url_domain()
                        .is_some_and(|d| d.to_ascii_lowercase().contains(&q)),
                    ClipKind::Code => "code".contains(&q),
                    ClipKind::Email => "email".contains(&q),
                    ClipKind::Text => false,
                }
            })
            .collect()
    }

    pub fn count_for(&self, filter: CategoryFilter) -> usize {
        self.items.iter().filter(|e| filter.matches(e)).count()
    }
}

fn import_from_cliphist(store: &mut ClipboardStore, dir: &Path, max_items: usize, limit: usize) -> usize {
    let Ok(out) = std::process::Command::new("cliphist").arg("list").output() else {
        return 0;
    };
    if !out.status.success() {
        return 0;
    }
    let listing = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = listing.lines().take(limit).collect();
    let base_now = now_unix();
    let mut imported = 0usize;

    // Import from oldest to newest so timestamps preserve cliphist's recency order
    for (idx, line) in lines.iter().enumerate().rev() {
        let Ok(mut child) = std::process::Command::new("cliphist")
            .arg("decode")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(line.as_bytes());
        }
        let Ok(decoded) = child.wait_with_output() else {
            continue;
        };
        if !decoded.status.success() || decoded.stdout.is_empty() {
            continue;
        }
        let ts = base_now.saturating_sub(idx as u64);
        if line.contains("[[ binary data ") {
            if store.add_image(dir, &decoded.stdout, ts, max_items).unwrap_or(false) {
                imported += 1;
            }
        } else if let Ok(text) = String::from_utf8(decoded.stdout)
            && store.add_text(text, ts, max_items)
        {
            imported += 1;
        }
    }
    imported
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn parse_colors_hex_rgb_hsl() {
        assert_eq!(parse_color("#3daee9"), Some(ColorRgba { r: 0x3d, g: 0xae, b: 0xe9, a: 255 }));
        assert_eq!(parse_color("#0f8"), Some(ColorRgba { r: 0x00, g: 0xff, b: 0x88, a: 255 }));
        assert_eq!(parse_color("#0f8a"), Some(ColorRgba { r: 0x00, g: 0xff, b: 0x88, a: 0xaa }));
        assert_eq!(parse_color("#11223380"), Some(ColorRgba { r: 0x11, g: 0x22, b: 0x33, a: 0x80 }));
        assert_eq!(parse_color("rgb(61, 174, 233)"), Some(ColorRgba { r: 61, g: 174, b: 233, a: 255 }));
        assert_eq!(parse_color("rgba(61, 174, 233, 0.5)"), Some(ColorRgba { r: 61, g: 174, b: 233, a: 128 }));
        assert_eq!(parse_color("hsl(0, 100%, 50%)"), Some(ColorRgba { r: 255, g: 0, b: 0, a: 255 }));
        assert_eq!(parse_color("hsl(120deg 100% 50% / 50%)"), Some(ColorRgba { r: 0, g: 255, b: 0, a: 128 }));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("rgb(300, 0, 0)"), None);
        assert_eq!(parse_color("hello world"), None);
    }

    #[test]
    fn content_classification() {
        assert!(matches!(classify_text("#3DAEE9"), ClipKind::Color(_)));
        assert_eq!(classify_text("https://maprisma.com/apps"), ClipKind::Link);
        assert_eq!(classify_text("http://localhost:8080/api"), ClipKind::Link);
        assert_eq!(classify_text("uwe@example.org"), ClipKind::Email);
        assert_eq!(classify_text("fn main() {\n    println!(\"hi\");\n}"), ClipKind::Code);
        assert_eq!(classify_text(r#"{"name": "Maprisma", "version": 1}"#), ClipKind::Code);
        assert_eq!(classify_text("cargo build --release"), ClipKind::Code);
        assert_eq!(classify_text("Meet at 15:00 near the cafe"), ClipKind::Text);
    }

    #[test]
    fn dedup_pinning_and_pruning_order() {
        let mut store = ClipboardStore::default();
        assert!(store.add_text("first".into(), 100, 2));
        assert!(store.add_text("second".into(), 101, 2));
        let first_id = store.items[1].id.clone();
        assert!(store.toggle_pin(&first_id));
        // Pinned item sorts above unpinned even with older timestamp
        assert_eq!(store.items[0].text, "first");
        assert!(store.items[0].pinned);

        // Adding 2 more unpinned items prunes oldest unpinned ("second"), never the pinned item
        store.add_text("third".into(), 102, 2);
        store.add_text("fourth".into(), 103, 2);
        assert_eq!(store.items.len(), 3); // 1 pinned + 2 unpinned
        assert_eq!(store.items[0].text, "first");
        assert_eq!(store.items[1].text, "fourth");
        assert_eq!(store.items[2].text, "third");

        // Re-copying "third" moves it to top of unpinned without duplicating
        store.add_text("third".into(), 200, 2);
        assert_eq!(store.items.len(), 3);
        assert_eq!(store.items[1].text, "third");

        // Re-copying pinned "first" keeps it pinned
        store.add_text("first".into(), 205, 2);
        assert!(store.items[0].pinned);
        assert_eq!(store.items[0].timestamp, 205);

        // Clear unpinned keeps pinned items
        store.clear_unpinned(None);
        assert_eq!(store.items.len(), 1);
        assert_eq!(store.items[0].text, "first");
    }

    #[test]
    fn search_and_category_filtering() {
        let mut store = ClipboardStore::default();
        store.add_text("#3DAEE9".into(), 10, 50);
        store.add_text("https://maprisma.com/clipboard".into(), 11, 50);
        store.add_text("fn hello() { println!(\"ok\"); }".into(), 12, 50);
        store.add_text("Simple shopping list".into(), 13, 50);
        let id = store.items[0].id.clone();
        store.toggle_pin(&id);

        assert_eq!(store.filtered("", CategoryFilter::All).len(), 4);
        assert_eq!(store.filtered("", CategoryFilter::Pinned).len(), 1);
        assert_eq!(store.filtered("", CategoryFilter::Colors).len(), 1);
        assert_eq!(store.filtered("", CategoryFilter::Links).len(), 1);
        assert_eq!(store.filtered("", CategoryFilter::Text).len(), 2); // text + code
        assert_eq!(store.filtered("maprisma.com", CategoryFilter::All).len(), 1);
        assert_eq!(store.filtered("3daee9", CategoryFilter::All).len(), 1);
        assert_eq!(store.filtered("rgb(61", CategoryFilter::All).len(), 1);
    }

    #[test]
    fn persistence_permissions_and_images() {
        let dir = std::env::temp_dir().join(format!("maprisma-clip-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let mut store = ClipboardStore::default();
        store.add_text("secret note".into(), 1000, 50);

        // Create a tiny 4x4 PNG in memory
        let mut png_bytes = Vec::new();
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([61, 174, 233, 255]));
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
            .unwrap();
        assert!(store.add_image(&dir, &png_bytes, 1001, 50).unwrap());
        store.save_to(&dir).unwrap();

        let hist_mode = std::fs::metadata(ClipboardStore::history_path(&dir))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(hist_mode, 0o600);

        let loaded = ClipboardStore::load_from(&dir);
        assert_eq!(loaded.items.len(), 2);
        let img_entry = loaded.items.iter().find(|e| matches!(e.kind, ClipKind::Image { .. })).unwrap();
        let img_path = img_entry.image_path(&dir).unwrap();
        assert!(img_path.exists());
        let img_mode = std::fs::metadata(&img_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(img_mode, 0o600);

        let mut loaded = loaded;
        loaded.clear_all(Some(&dir));
        assert!(!img_path.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
