use super::*;

pub static CONFIG_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| dirs::config_dir().unwrap().join(".tablica_cfg"));

// Config = "key=value" lines. Parsing is split from file I/O so it can be tested.
pub fn parse_get(t: &str, k: &str) -> Option<String> {
    t.lines().find_map(|l| {
        let (key, v) = l.split_once('=')?;
        (key.trim() == k).then(|| v.trim().to_string())
    })
}

pub fn parse_set(t: &str, k: &str, v: &str) -> String {
    let mut out: String = t
        .lines()
        .filter(|l| l.split_once('=').is_none_or(|(key, _)| key.trim() != k))
        .map(|l| format!("{l}\n"))
        .collect();
    out += &format!("{k}={v}\n");
    out
}

pub fn cfg_get(k: &str) -> Option<String> {
    parse_get(&fs::read_to_string(&*CONFIG_PATH).ok()?, k)
}

pub fn cfg_set(k: &str, v: &str) {
    let old = fs::read_to_string(&*CONFIG_PATH).unwrap_or_default();
    let _ = fs::write(&*CONFIG_PATH, parse_set(&old, k, v));
}
