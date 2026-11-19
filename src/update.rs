use super::*;

// Update check: read Cargo.toml of the repo from raw.githubusercontent.com; if its version is
// newer than ours, show a popup. Network errors are silent.
const HOST: &str = "raw.githubusercontent.com";
const PATH: &str = "/eye-wave/8x20/main/Cargo.toml";
const PAGE: &str = "https://nightly.link/eye-wave/8x20/workflows/build/main"; // opened by the popup's "Pobierz"

// `version = "x.y.z"` under [package]
fn remote_version(toml: &str) -> Option<&str> {
    toml.lines()
        .skip_while(|l| l.trim() != "[package]")
        .skip(1)
        .take_while(|l| !l.starts_with('['))
        .find_map(|l| {
            let v = l.strip_prefix("version")?.trim_start().strip_prefix('=')?;
            v.trim().strip_prefix('"')?.split('"').next()
        })
}

// ([major, minor, patch], is_release): "0.2.10" > "0.2.10-dev"; other suffixes are not ordered.
fn parse(v: &str) -> ([u32; 3], bool) {
    let (n, pre) = v.split_once('-').map_or((v, false), |(n, _)| (n, true));
    let mut p = n.split('.').map(|x| x.parse().unwrap_or(0));
    let mut next = || p.next().unwrap_or(0);
    ([next(), next(), next()], !pre)
}

// Open a URL in the default browser.
// `start` via cmd (no console flash) / xdg-open; no ShellExecute binding to own.
fn open(url: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn();
    }
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

static BUSY: AtomicBool = AtomicBool::new(false);

// Setting "Sprawdzaj aktualizacje" (default on): the startup check.
pub fn auto_on() -> bool {
    cfg_get("updates").as_deref() != Some("0")
}

// `manual` (the button) also reports "up to date" and failures; the startup check stays silent unless
// there is something new.
pub fn check(manual: bool) {
    if BUSY.swap(true, Relaxed) {
        return;
    }
    let me = env!("CARGO_PKG_VERSION");
    bg(
        || {
            https_get(HOST, PATH)
                .ok()
                .and_then(|t| remote_version(&t).map(str::to_owned))
        },
        move |v| {
            BUSY.store(false, Relaxed);
            match v {
                Some(v) if parse(&v) > parse(me) => {
                    let msg = format!("Dostępna nowa wersja: {v}\n(używasz {me})\n\nPobrać teraz?");
                    if ask(&msg, &["Pobierz", "Później"]) == Some(0) {
                        open(PAGE);
                    }
                }
                Some(_) if manual => about(&format!("Używasz najnowszej wersji ({me}).")),
                None if manual => {
                    about("Nie udało się sprawdzić aktualizacji.\nSprawdź połączenie z internetem.")
                }
                _ => {}
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        let t =
            "[package]\nname = \"x\"\nversion = \"0.3.1\" # c\n\n[dependencies]\nversion = \"9\"\n";
        assert_eq!(remote_version(t), Some("0.3.1"));
        assert_eq!(remote_version("[dependencies]\nversion = \"9\""), None);
        assert!(parse("0.2.10") > parse("0.2.10-dev"));
        assert!(parse("0.2.11-dev") > parse("0.2.10"));
        assert!(parse("0.10.0") > parse("0.9.9"));
        assert!(parse("0.2.10-dev") <= parse("0.2.10-dev"));
    }
}
