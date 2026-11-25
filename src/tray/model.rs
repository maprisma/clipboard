//! Tray menu model and tooltip formatting for Maprisma Clipboard.
use maprisma_clipboard::i18n::{choices, tr, trf};
use maprisma_clipboard::store::ClipEntry;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Activate,
    Open,
    TogglePause,
    CopyClip(String),
    ClearUnpinned,
    Language(usize),
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: i32,
    pub label: String,
    pub enabled: bool,
    pub separator: bool,
    pub children: Vec<Item>,
    pub action: Option<Action>,
}

impl Item {
    pub fn find(&self, id: i32) -> Option<&Self> {
        if self.id == id {
            Some(self)
        } else {
            self.children.iter().find_map(|c| c.find(id))
        }
    }

    pub fn activate(&self, id: i32) -> Option<Action> {
        self.find(id).filter(|i| i.enabled).and_then(|i| i.action.clone())
    }
}

pub fn tooltip(count: usize, paused: bool) -> String {
    match (count, paused) {
        (0, false) => tr("Empty · Recording"),
        (0, true) => tr("Empty · Paused"),
        (1, false) => tr("1 item · Recording"),
        (1, true) => tr("1 item · Paused"),
        (n, false) => trf("{0} items · Recording", &[n.to_string()]),
        (n, true) => trf("{0} items · Paused", &[n.to_string()]),
    }
}

pub fn menu(entries: &[ClipEntry], paused: bool, lang_error: bool) -> Item {
    let item = |id, label: String, action| Item {
        id,
        label,
        enabled: true,
        separator: false,
        children: vec![],
        action,
    };
    let sep = |id| Item {
        id,
        label: String::new(),
        enabled: true,
        separator: true,
        children: vec![],
        action: None,
    };

    let mut root = item(0, String::new(), None);
    let pause_label = if paused {
        tr("Resume recording")
    } else {
        tr("Pause recording")
    };

    let mut children = vec![
        item(1, tr("Open Maprisma Clipboard"), Some(Action::Open)),
        item(2, pause_label, Some(Action::TogglePause)),
        sep(3),
    ];

    let recent: Vec<&ClipEntry> = entries.iter().take(5).collect();
    if recent.is_empty() {
        children.push(Item {
            id: 20,
            label: tr("No recent clips"),
            enabled: false,
            separator: false,
            children: vec![],
            action: None,
        });
    } else {
        children.push(Item {
            id: 19,
            label: tr("Recent clips"),
            enabled: false,
            separator: false,
            children: vec![],
            action: None,
        });
        for (idx, entry) in recent.iter().enumerate() {
            let prefix = if entry.pinned { "★ " } else { "" };
            let preview = entry.preview_line(46);
            children.push(item(
                20 + idx as i32,
                format!("{prefix}{preview}"),
                Some(Action::CopyClip(entry.id.clone())),
            ));
        }
    }

    children.push(sep(30));
    children.push(item(31, tr("Clear unpinned"), Some(Action::ClearUnpinned)));

    let mut languages = item(40, tr("Language"), None);
    languages.children = choices()
        .iter()
        .enumerate()
        .map(|(i, (_, label))| item(100 + i as i32, label.clone(), Some(Action::Language(i))))
        .collect();
    children.push(languages);

    if lang_error {
        children.push(Item {
            id: 49,
            label: tr("Could not save language setting."),
            enabled: false,
            separator: false,
            children: vec![],
            action: None,
        });
    }

    children.push(sep(50));
    children.push(item(51, tr("Quit tray"), Some(Action::Quit)));

    root.children = children;
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use maprisma_clipboard::store::ClipKind;

    #[test]
    fn tray_menu_actions_and_recent_clips() {
        let clip = ClipEntry {
            id: "abc123".into(),
            kind: ClipKind::Text,
            text: "Hello Maprisma".into(),
            pinned: true,
            timestamp: 100,
            chars: 14,
            lines: 1,
        };
        let m = menu(&[clip], false, false);
        assert_eq!(m.activate(1), Some(Action::Open));
        assert_eq!(m.activate(2), Some(Action::TogglePause));
        assert_eq!(m.activate(20), Some(Action::CopyClip("abc123".into())));
        assert_eq!(m.activate(31), Some(Action::ClearUnpinned));
        assert_eq!(m.activate(100), Some(Action::Language(0)));
        assert_eq!(m.activate(51), Some(Action::Quit));
        assert!(m.find(20).unwrap().label.contains("Hello Maprisma"));
    }

    #[test]
    fn tray_tooltip_counts_and_pause_state() {
        assert_eq!(tooltip(0, false), "Empty · Recording");
        assert_eq!(tooltip(0, true), "Empty · Paused");
        assert_eq!(tooltip(1, false), "1 item · Recording");
        assert_eq!(tooltip(4, true), "4 items · Paused");
    }
}
