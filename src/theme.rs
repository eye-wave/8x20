use std::cell::{Cell, RefCell};

use fltk::{app, enums::Color, image};

// ---- themes ----------------------------------------------------------------------------------
// The theme list lives in themes.yaml; build.rs (themes_build.rs) turns it into the `THEMES` table
// included below. Here: the types it fills in, and the runtime.

// Every colour the UI uses, per theme.
pub struct Colors {
    pub label: u32,     // COM label + status message (on the dark status bar)
    pub panel: u32,     // "Dodatki" overlay panel
    pub accent: u32,    // selected thumbnail in "Dodatki"
    pub rule: u32,      // separator line in "Dodatki"
    pub scrim: u32,     // dimming lines behind the overlay
    pub page_line: u32, // page separator line in the editor
    pub marks: u32,     // · / ¶ for invisible characters
    pub text: u32,      // text in the editors
    pub text_bg: u32,   // editor background
    pub page_info: u32, // "K: 1  W: 1" and "Strona - 001" above the editors
}

type Day = (u32, u32); // (month, day)

pub struct Theme {
    pub name: &'static str, // shown to the user
    pub colors: Colors,
    pub map: &'static [(u32, u32)], // SVG colour -> this theme's colour (empty = SVGs as drawn)
    pub bg: &'static [u8],          // the backdrop, the only per-theme file
    pub season: Option<(Day, Day)>, // first/last day it activates by itself, inclusive; may wrap over New Year
}

// The window icon is the same in every theme.
pub const ICON: &[u8] = include_bytes!("../tex/icon.png");

// "nowy_off" -> svg source, ... (<n>_off/_on per button, sep, status_dot_on/off)
macro_rules! svgs {
    ($($n:literal),*) => {
        &[
            $(
                (concat!($n, "_off"), include_str!(concat!("../tex/", $n, "_off.svg"))),
                (concat!($n, "_on"), include_str!(concat!("../tex/", $n, "_on.svg"))),
            )*
            ("sep", include_str!("../tex/sep.svg")),
            ("status_dot_on", include_str!("../tex/status_dot_on.svg")),
            ("status_dot_off", include_str!("../tex/status_dot_off.svg")),
        ]
    };
}
static SVG: &[(&str, &str)] = svgs!(
    "min",
    "close",
    "nowy",
    "otworz",
    "zapisz",
    "odczytaj",
    "wyslij",
    "przerwij",
    "dodatki",
    "lewo",
    "srodek",
    "prawo",
    "justuj",
    "pion_gora",
    "pion_srodek",
    "znaki",
    "szukaj",
    "info"
);

// key -> theme, in themes.yaml order (the first is the default)
include!(concat!(env!("OUT_DIR"), "/themes.rs"));

type Skin = Box<dyn FnMut(&Theme)>;
thread_local! {
    pub static CUR: Cell<usize> = const { Cell::new(0) }; // index into THEMES
    pub static SKINS: RefCell<Vec<Skin>> = const { RefCell::new(Vec::new()) }; // re-skin hooks of live widgets
}

pub fn theme() -> &'static Theme {
    &THEMES[CUR.get()].1
}

// Register how a widget re-skins itself when the theme changes.
pub fn skin(f: impl FnMut(&Theme) + 'static) {
    SKINS.with(|s| s.borrow_mut().push(Box::new(f)));
}

// FLTK's own gray (dialogs, menus, scrollbars, plain buttons) -> the theme's panel / dark text colour.
pub fn tint() {
    let c = &theme().colors;
    let (r, g, b) = Color::from_hex(c.panel).to_rgb();
    app::background(r, g, b);
    let (r, g, b) = Color::from_hex(c.scrim).to_rgb();
    app::foreground(r, g, b);
}

pub fn apply_theme(i: usize) {
    CUR.set(i);
    tint();
    SKINS.with(|s| s.borrow_mut().iter_mut().for_each(|f| f(theme())));
    app::redraw();
}

// Swap colours in an SVG per the theme's map. Single pass over "#rrggbb" (any case); anything not
// in the map (gradient ids, "none") is left alone.
fn recolor(svg: &str, map: &[(u32, u32)]) -> String {
    if map.is_empty() {
        return svg.into();
    }
    let svg = svg.replace("\"white\"", "\"#FFFFFF\"");
    let (mut out, mut rest) = (String::with_capacity(svg.len()), svg.as_str());
    while let Some(i) = rest.find('#') {
        out += &rest[..i + 1];
        rest = &rest[i + 1..];
        let Some((hex, tail)) = rest.split_at_checked(6) else {
            break;
        };
        let c = u32::from_str_radix(hex, 16)
            .ok()
            .filter(|_| hex.bytes().all(|b| b.is_ascii_hexdigit()));
        if let Some((_, to)) = c.and_then(|c| map.iter().find(|(from, _)| *from == c)) {
            out += &format!("{to:06X}");
            rest = tail;
        }
    }
    out + rest
}

pub fn img(t: &Theme, name: &str) -> image::SvgImage {
    let d = SVG.iter().find(|(n, _)| *n == name).map(|(_, d)| *d);
    svg(
        name,
        &recolor(d.unwrap_or_else(|| panic!("tex: no {name}")), t.map),
    )
}

// Today's (month, day), UTC (no date crate: Hinnant's civil-from-days).
pub fn today() -> (u32, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let z = (secs / 86400) as i64 + 719_468;
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (m as u32, (doy - (153 * mp + 2) / 5 + 1) as u32)
}

pub fn seasonal_on() -> bool {
    crate::cfg_get("seasonal").as_deref() != Some("0")
}

// Theme to use now: the seasonal one if enabled and in season, else the saved choice, else the first.
pub fn pick_theme() -> usize {
    let by = |k: &str| THEMES.iter().position(|(n, _)| *n == k);
    let t = today();
    let season = THEMES.iter().position(|(_, th)| {
        th.season.is_some_and(|(a, b)| {
            if a <= b {
                a <= t && t <= b
            } else {
                t >= a || t <= b
            }
        })
    });
    season
        .filter(|_| seasonal_on())
        .or_else(|| crate::cfg_get("theme").and_then(|k| by(&k)))
        .unwrap_or(0)
}

pub fn png(name: &str, d: &[u8]) -> image::PngImage {
    image::PngImage::from_data(d).unwrap_or_else(|e| panic!("tex/{name}.png: {e}"))
}

// The window icon (the OS needs a raster) and the backdrop (too fancy for nanosvg) stay PNG; everything else in tex/ is SVG.
pub fn svg(name: &str, d: &str) -> image::SvgImage {
    image::SvgImage::from_data(d).unwrap_or_else(|e| panic!("tex/{name}.svg: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colours(svg: &str) -> impl Iterator<Item = u32> + '_ {
        svg.match_indices('#')
            .filter_map(|(i, _)| svg.get(i + 1..i + 7))
            .filter(|h| h.bytes().all(|b| b.is_ascii_hexdigit()))
            .filter_map(|h| u32::from_str_radix(h, 16).ok())
    }

    #[test]
    fn recolor_swaps_hex_and_white_only() {
        let m = [(0xB9A56A, 1), (0xFFFFFF, 2)];
        assert_eq!(
            recolor("a=\"#b9a56a\" b=\"white\" c=\"none\" url(#paint0)", &m),
            "a=\"#000001\" b=\"#000002\" c=\"none\" url(#paint0)"
        );
        assert_eq!(recolor("fill=\"#b9a56a\"", &[]), "fill=\"#b9a56a\"");
    }

    #[test]
    fn themes_map_every_svg_colour() {
        for (key, t) in THEMES.iter().filter(|(_, t)| !t.map.is_empty()) {
            for (name, d) in SVG {
                for c in colours(&d.replace("\"white\"", "\"#FFFFFF\"")) {
                    assert!(
                        t.map.iter().any(|&(f, _)| f == c),
                        "{key}: {name} has unmapped #{c:06X}"
                    );
                }
            }
        }
    }
}
