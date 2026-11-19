use super::*;

// Update check: read Cargo.toml of the repo from raw.githubusercontent.com; if its version is
// newer than ours, show a popup. Network errors are silent.
const HOST: &str = "raw.githubusercontent.com";
const PATH: &str = "/eye-wave/8x20/main/Cargo.toml";
const PAGE: &str = "https://nightly.link/eye-wave/8x20/workflows/build/main";

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

pub fn check() {
    let me = env!("CARGO_PKG_VERSION");
    bg(
        || {
            https_get(HOST, PATH)
                .ok()
                .and_then(|t| remote_version(&t).map(str::to_owned))
        },
        move |v| {
            if let Some(v) = v.filter(|v| parse(v) > parse(me)) {
                about(&format!(
                    "Dostępna nowa wersja: {v}\n(używasz {me})\n\n{PAGE}"
                ));
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
