use super::*;

// Update check: read Cargo.toml of the repo from raw.githubusercontent.com; if its version is
// newer than ours, show a popup. Network errors are silent.
const HOST: &str = "raw.githubusercontent.com";
const PATH: &str = "/eye-wave/8x20/main/Cargo.toml";
const PAGE: &str = "https://github.com/eye-wave/8x20/releases/latest"; // fallback if the in-app update fails
// Latest GitHub release (public, no token; Actions artifacts would need one).
// Matches the CI matrix names: windows-10-64 is the default (rfd) build, win7 builds drop it.
const FLAVOUR: &str = if cfg!(windows) {
    if cfg!(feature = "rfd") {
        "windows-10-64"
    } else if cfg!(target_pointer_width = "64") {
        "windows-7-64"
    } else {
        "windows-7-32"
    }
} else if cfg!(target_pointer_width = "64") {
    "linux-64"
} else {
    "linux-32"
};
const API: &str = "https://api.github.com/repos/eye-wave/8x20/releases/latest";

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

// Download this build's binary from the latest release and swap it in for the running exe. A running exe can be
// renamed (even on Windows) but not overwritten, so: exe -> exe.old, new -> exe, `.old` is deleted
// on the next start.
fn install() -> Result<(), String> {
    let e = |e: &dyn std::fmt::Display| e.to_string();
    let get = |url: &str| {
        let r = minreq::get(url)
            .with_header("User-Agent", "tablica")
            .with_timeout(120)
            .send()
            .map_err(|x| e(&x))?;
        if r.status_code == 200 {
            Ok(r)
        } else {
            Err(format!("HTTP {}", r.status_code))
        }
    };
    // release asset of this exact build flavour (see build.yml: tablica-<name>[.exe])
    let name = format!(
        "tablica-{FLAVOUR}{}",
        if cfg!(windows) { ".exe" } else { "" }
    );
    let json = get(API)?;
    let url = regex_lite::Regex::new(r#""browser_download_url"\s*:\s*"([^"]+)""#)
        .unwrap()
        .captures_iter(json.as_str().map_err(|x| e(&x))?)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_owned()))
        .find(|u| u.rsplit('/').next() == Some(&name))
        .ok_or("no release asset")?;
    let data = get(&url)?.into_bytes();
    let exe = std::env::current_exe().map_err(|x| e(&x))?;
    let old = exe.with_extension("old");
    let _ = fs::remove_file(&old);
    fs::rename(&exe, &old).map_err(|x| e(&x))?;
    if let Err(x) = fs::write(&exe, data) {
        let _ = fs::rename(&old, &exe); // put the old one back
        return Err(e(&x));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&exe, fs::Permissions::from_mode(0o755));
    }
    std::process::Command::new(&exe)
        .spawn()
        .map_err(|x| e(&x))?;
    Ok(())
}

// Startup: remove the previous exe left behind by an update.
pub fn cleanup() {
    if let Ok(e) = std::env::current_exe() {
        let _ = fs::remove_file(e.with_extension("old"));
    }
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
                // -dev versions are never released
                Some(v) if !v.contains('-') && parse(&v) > parse(me) => {
                    let msg = format!(
                        "Dostępna nowa wersja: {v}\n(używasz {me})\n\nPobrać i zainstalować teraz?"
                    );
                    if ask(&msg, &["Aktualizuj", "Później"]) == Some(0) {
                        bg(install, |r| match r {
                            Ok(()) => std::process::exit(0), // new exe already started
                            Err(_) => {
                                about("Nie udało się zaktualizować.\nOtwieram stronę pobierania.");
                                open(PAGE);
                            }
                        });
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
