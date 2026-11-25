//! Bundled 8-language catalogs and shared UI/app preferences in ~/.config/maprisma-clipboard/config.json.
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{LazyLock, RwLock},
};

pub const LANGUAGES: [(&str, &str); 8] = [
    ("en", "English"),
    ("de", "Deutsch"),
    ("fr", "Français"),
    ("es", "Español"),
    ("it", "Italiano"),
    ("pt", "Português"),
    ("nl", "Nederlands"),
    ("pl", "Polski"),
];

static CATALOGS: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../translations/bundled.json")).expect("bundled catalogs")
});
static CONFIG_CACHE: RwLock<Option<(PathBuf, Value)>> = RwLock::new(None);
static SYSTEM_LANG: LazyLock<&'static str> = LazyLock::new(system);

#[cfg(test)]
std::thread_local! {
    static TEST_PATH: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

pub fn system_language(values: [&str; 3]) -> &'static str {
    let first = values.into_iter().find(|v| !v.is_empty()).unwrap_or("");
    let code = first.split(['_', '.', '@', '-']).next().unwrap_or("").to_lowercase();
    LANGUAGES.iter().find(|(c, _)| *c == code).map_or("en", |(c, _)| c)
}

pub fn system() -> &'static str {
    let values = ["LC_ALL", "LC_MESSAGES", "LANG"].map(|k| std::env::var(k).unwrap_or_default());
    system_language([&values[0], &values[1], &values[2]])
}

pub fn config_path() -> PathBuf {
    #[cfg(test)]
    if let Some(p) = TEST_PATH.with(|cell| cell.borrow().clone()) {
        return p;
    }
    crate::config_home().join("maprisma-clipboard/config.json")
}

fn read_disk(p: &Path) -> Value {
    std::fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
        .unwrap_or(Value::Null)
}

/// Reads config.json once from disk and updates the in-memory preference cache.
pub fn reload_prefs() {
    let p = config_path();
    let val = read_disk(&p);
    let mut guard = CONFIG_CACHE.write().unwrap_or_else(|e| e.into_inner());
    *guard = Some((p, val));
}

fn with_cached_config<T>(f: impl FnOnce(&Value) -> T) -> T {
    let p = config_path();
    {
        let guard = CONFIG_CACHE.read().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_path, val)) = guard.as_ref()
            && *cached_path == p
        {
            return f(val);
        }
    }
    let val = read_disk(&p);
    let out = f(&val);
    let mut guard = CONFIG_CACHE.write().unwrap_or_else(|e| e.into_inner());
    *guard = Some((p, val));
    out
}

pub fn active_code() -> &'static str {
    with_cached_config(|v| {
        v["language"]
            .as_str()
            .and_then(|c| LANGUAGES.iter().find(|(code, _)| *code == c).map(|(code, _)| *code))
    })
    .unwrap_or_else(|| *SYSTEM_LANG)
}

pub fn setting() -> String {
    with_cached_config(|v| {
        v["language"]
            .as_str()
            .filter(|c| *c == "auto" || LANGUAGES.iter().any(|(code, _)| code == c))
            .unwrap_or("auto")
            .to_owned()
    })
}

pub fn save(code: &str) -> std::io::Result<()> {
    if code != "auto" && !LANGUAGES.iter().any(|(c, _)| *c == code) {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    write_pref("language", Some(code.into()))
}

pub fn read_pref(key: &str) -> Option<Value> {
    with_cached_config(|v| v.get(key).cloned())
}

pub fn write_pref(key: &str, value: Option<Value>) -> std::io::Result<()> {
    let path = config_path();
    let mut config = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    match value {
        Some(value) => config[key] = value,
        None => {
            if let Some(o) = config.as_object_mut() {
                o.remove(key);
            }
        }
    }
    let data = serde_json::to_string_pretty(&config)? + "\n";
    crate::write_atomic(&path, data.as_bytes())?;
    let mut guard = CONFIG_CACHE.write().unwrap_or_else(|e| e.into_inner());
    *guard = Some((path, config));
    Ok(())
}

pub fn translate(code: &str, key: &str) -> String {
    let value = &CATALOGS[code][key];
    value
        .as_str()
        .or_else(|| value.as_array().and_then(|a| a.last()).and_then(Value::as_str))
        .unwrap_or(key)
        .to_owned()
}

pub fn tr(key: &str) -> String {
    translate(active_code(), key)
}

pub fn trf(key: &str, args: &[String]) -> String {
    let code = active_code();
    let value = &CATALOGS[code][key];
    let text = if code == "pl" && value.is_array() {
        let n = args.first().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let few = (2..=4).contains(&(n % 10)) && !(12..=14).contains(&(n % 100));
        value[if few { 0 } else { 1 }].as_str().unwrap_or(key).to_owned()
    } else {
        translate(code, key)
    };
    format(&text, args)
}

fn format(text: &str, args: &[String]) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some(end) = rest.find('}')
            && let Ok(i) = rest[1..end].parse::<usize>()
            && let Some(arg) = args.get(i)
        {
            result.push_str(arg);
            rest = &rest[end + 1..];
            continue;
        }
        result.push('{');
        rest = &rest[1..];
    }
    result.push_str(rest);
    result
}

pub fn choices() -> Vec<(String, String)> {
    let name = LANGUAGES.iter().find(|(c, _)| *c == *SYSTEM_LANG).unwrap().1;
    let mut choices = vec![("auto".into(), trf("System ({0})", &[name.into()]))];
    choices.extend(LANGUAGES.iter().map(|(c, n)| (c.to_string(), n.to_string())));
    choices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_precedence_and_fallback() {
        for (values, expected) in [
            (["de_DE.UTF-8", "fr", "pl"], "de"),
            (["", "fr-FR", "de"], "fr"),
            (["", "", "pt_BR.UTF-8"], "pt"),
            (["zz_ZZ", "de", "fr"], "en"),
            (["C.UTF-8", "de", "fr"], "en"),
            (["POSIX", "de", "fr"], "en"),
            (["", "", ""], "en"),
            (["PL_pl@x", "", ""], "pl"),
        ] {
            assert_eq!(system_language(values), expected);
        }
    }

    #[test]
    fn all_eight_languages_cover_all_english_keys() {
        let en = CATALOGS["en"].as_object().expect("en catalog");
        assert!(!en.is_empty());
        for (code, _) in LANGUAGES {
            let cat = CATALOGS[code].as_object().unwrap_or_else(|| panic!("missing catalog {code}"));
            for key in en.keys() {
                assert!(cat.contains_key(key), "missing key {key:?} in language {code}");
            }
        }
    }

    #[test]
    fn fallback_and_placeholder_safety() {
        assert_eq!(translate("unknown", "Unlisted"), "Unlisted");
        assert_eq!(translate("de", "Language"), "Sprache");
        assert_eq!(translate("fr", "Settings"), "Paramètres");
        assert_eq!(format("{0} {1}", &["{1}".into(), "Clip".into()]), "{1} Clip");
    }

    #[test]
    fn config_cache_updates_on_save_write_pref_and_reload() {
        struct PathGuard(PathBuf);
        impl Drop for PathGuard {
            fn drop(&mut self) {
                TEST_PATH.with(|cell| *cell.borrow_mut() = None);
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let dir = std::env::temp_dir().join(format!("clip-i18n-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = dir.join("config.json");
        let _guard = PathGuard(dir);
        TEST_PATH.with(|cell| *cell.borrow_mut() = Some(cfg.clone()));

        reload_prefs();
        assert_eq!(setting(), "auto");
        assert_eq!(read_pref("corners"), None);

        assert!(save("xx").is_err());
        save("de").unwrap();
        assert_eq!(setting(), "de");
        assert_eq!(tr("Language"), "Sprache");

        write_pref("corners", Some(serde_json::json!("rounded"))).unwrap();
        assert_eq!(read_pref("corners"), Some(serde_json::json!("rounded")));

        std::fs::write(&cfg, r#"{"language":"fr","corners":"square"}"#).unwrap();
        assert_eq!(setting(), "de");
        reload_prefs();
        assert_eq!(setting(), "fr");
        assert_eq!(read_pref("corners"), Some(serde_json::json!("square")));

        write_pref("corners", None).unwrap();
        assert_eq!(read_pref("corners"), None);
        assert_eq!(setting(), "fr");
    }
}
