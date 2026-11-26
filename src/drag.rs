//! What a clip offers when its card is dragged out of the window (into a terminal, an editor, a
//! file manager…): text clips as plain UTF-8 text, image clips as the image itself plus its file
//! as a `file://` URI. The image file is read once, when the drag starts.

use std::borrow::Cow;
use std::path::Path;

use cosmic::iced::clipboard::mime::AsMimeTypes;

/// The text kinds receivers ask for (Wayland apps, XWayland apps via the X atoms).
const TEXT_MIMES: [&str; 5] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING", "TEXT"];

/// Images larger than this are offered as their file only (no copy of the bytes in memory).
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum DragData {
    Text(String),
    Image { mime: &'static str, bytes: Option<Vec<u8>>, uri: String },
}

impl DragData {
    /// An image clip's file: its type from the extension, its bytes when the file is a regular
    /// file of sane size, and its `file://` URI (path bytes percent-encoded).
    pub fn image(path: &Path) -> Self {
        let mime = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            Some("bmp") => "image/bmp",
            _ => "image/png",
        };
        let bytes = std::fs::symlink_metadata(path)
            .ok()
            .filter(|m| m.is_file() && m.len() <= MAX_IMAGE_BYTES)
            .and_then(|_| std::fs::read(path).ok());
        Self::Image { mime, bytes, uri: file_uri(path) }
    }
}

/// `file:///…` with every byte outside the unreserved set and `/` percent-encoded.
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

impl AsMimeTypes for DragData {
    fn available(&self) -> Cow<'static, [String]> {
        let list: Vec<String> = match self {
            Self::Text(_) => TEXT_MIMES.iter().map(|m| (*m).to_string()).collect(),
            Self::Image { mime, bytes, .. } => {
                let mut list = Vec::new();
                if bytes.is_some() {
                    list.push((*mime).to_string());
                }
                list.push("text/uri-list".to_string());
                list
            }
        };
        Cow::Owned(list)
    }

    fn as_bytes(&self, mime_type: &str) -> Option<Cow<'static, [u8]>> {
        match self {
            Self::Text(text) => TEXT_MIMES.contains(&mime_type).then(|| Cow::Owned(text.as_bytes().to_vec())),
            Self::Image { mime, bytes, uri } => {
                if mime_type == "text/uri-list" {
                    Some(Cow::Owned(format!("{uri}\r\n").into_bytes()))
                } else if mime_type == *mime {
                    bytes.clone().map(Cow::Owned)
                } else {
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_offered_as_plain_text_only() {
        let d = DragData::Text("echo $(rm -rf ~)\n".into());
        assert!(d.available().iter().any(|m| m == "text/plain;charset=utf-8"));
        assert_eq!(d.as_bytes("text/plain").as_deref(), Some("echo $(rm -rf ~)\n".as_bytes()));
        assert_eq!(d.as_bytes("text/uri-list"), None);
        assert_eq!(d.as_bytes("text/html"), None);
    }

    #[test]
    fn image_offers_bytes_and_its_file() {
        let dir = std::env::temp_dir().join(format!("clip-drag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a b%.png");
        std::fs::write(&path, b"\x89PNG").unwrap();
        let d = DragData::image(&path);
        assert_eq!(d.available().as_ref(), ["image/png".to_string(), "text/uri-list".to_string()]);
        assert_eq!(d.as_bytes("image/png").as_deref(), Some(&b"\x89PNG"[..]));
        let uri = String::from_utf8(d.as_bytes("text/uri-list").unwrap().into_owned()).unwrap();
        assert!(uri.starts_with("file:///") && uri.ends_with("a%20b%25.png\r\n"), "{uri}");
        // A symlink or a missing file: only the URI, no bytes read through it.
        let link = dir.join("link.png");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(DragData::image(&link).available().as_ref(), ["text/uri-list".to_string()]);
        assert_eq!(DragData::image(&dir.join("gone.jpg")).as_bytes("image/jpeg"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn uri_escapes_control_and_reserved_bytes() {
        assert_eq!(file_uri(Path::new("/x/a\r\nb#?.png")), "file:///x/a%0D%0Ab%23%3F.png");
    }
}
