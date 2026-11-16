// Build-time half of the theme system: themes.yaml -> $OUT_DIR/themes.rs (the `THEMES` table,
// include!d by src/theme.rs). serde + serde_yaml are build-dependencies only, so neither ends up
// in the binary. Hook it into build.rs: `mod themes_build;` and `themes_build::run();`.
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::PathBuf,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Colors {
    label: u32,
    panel: u32,
    accent: u32,
    rule: u32,
    scrim: u32,
    page_line: u32,
    marks: u32,
    text: u32,
    text_bg: u32,
    page_info: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Season {
    from: String,
    to: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Theme {
    key: String,
    name: String,
    bg: String,
    colors: Colors,
    season: Option<Season>,
    #[serde(default)]
    recolor: BTreeMap<u32, u32>,
}

// "DD-MM" -> (month, day)
fn md(s: &str) -> (u32, u32) {
    s.split_once('-')
        .and_then(|(d, m)| Some((m.parse().ok()?, d.parse().ok()?)))
        .filter(|&(m, d)| (1..=12).contains(&m) && (1..=31).contains(&d))
        .unwrap_or_else(|| panic!("themes.yaml: bad date {s:?}, want DD-MM"))
}

// Every #rrggbb in tex/*.svg, plus "white".
fn svg_colours() -> BTreeSet<u32> {
    let mut set = BTreeSet::from([0xFFFFFF]);
    for e in fs::read_dir("tex").unwrap().flatten() {
        if e.path().extension().is_some_and(|x| x == "svg") {
            let s = fs::read_to_string(e.path()).unwrap();
            set.extend(
                s.match_indices('#')
                    .filter_map(|(i, _)| s.get(i + 1..i + 7))
                    .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
                    .filter_map(|h| u32::from_str_radix(h, 16).ok()),
            );
        }
    }
    set
}

pub fn run() {
    println!("cargo:rerun-if-changed=themes.yaml");
    println!("cargo:rerun-if-changed=tex");
    let src = fs::read_to_string("themes.yaml").expect("themes.yaml");
    let themes: Vec<Theme> =
        serde_yaml::from_str(&src).unwrap_or_else(|e| panic!("themes.yaml: {e}"));
    assert!(!themes.is_empty(), "themes.yaml: no themes");
    let known = svg_colours();

    let mut out = String::from("pub static THEMES: &[(&str, Theme)] = &[\n");
    for t in &themes {
        for from in t.recolor.keys() {
            assert!(
                known.contains(from),
                "themes.yaml: {}: 0x{from:06X} is not used in tex/*.svg",
                t.key
            );
        }
        let c = &t.colors;
        let map: Vec<_> = t
            .recolor
            .iter()
            .map(|(f, to)| format!("(0x{f:06X}, 0x{to:06X})"))
            .collect();
        let season = t.season.as_ref().map_or("None".into(), |s| {
            format!("Some(({:?}, {:?}))", md(&s.from), md(&s.to))
        });
        out += &format!(
            "    ({:?}, Theme {{ name: {:?}, colors: Colors {{ label: 0x{:06X}, panel: 0x{:06X}, accent: 0x{:06X}, \
             rule: 0x{:06X}, scrim: 0x{:06X}, page_line: 0x{:06X}, marks: 0x{:06X}, text: 0x{:06X}, text_bg: 0x{:06X}, page_info: 0x{:06X} }}, map: &[{}], \
             bg: include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/tex/\", {:?})), season: {season} }}),\n",
            t.key,
            t.name,
            c.label,
            c.panel,
            c.accent,
            c.rule,
            c.scrim,
            c.page_line,
            c.marks,
            c.text,
            c.text_bg,
            c.page_info,
            map.join(", "),
            t.bg,
        );
    }
    out += "];\n";
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("themes.rs"),
        out,
    )
    .unwrap();
}
