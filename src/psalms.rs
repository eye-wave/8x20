use crate::theme::civil;

use super::*;
use std::sync::atomic::{AtomicU32, AtomicUsize};

// "Psalmy na miesiąc": pick year + month + output format, download, save. The three `todo` hooks below
// (fetch, render, save) are the only places that still need real code.

#[derive(Clone, Copy)]
pub enum Format {
    Word2003,
    Tablica,
}

// (format, texture name in tex/ (63x71), file extension, caption drawn on the placeholder texture)
const FORMATS: &[(Format, &str, &str, &str)] = &[
    (Format::Word2003, "fmt_word", "xml", "Word"),
    (Format::Tablica, "fmt_txt", "txt", "TXT"),
];

static FORMAT: AtomicUsize = AtomicUsize::new(0); // index into FORMATS
static BUSY: AtomicBool = AtomicBool::new(false);
static DONE: AtomicU32 = AtomicU32::new(0);
static TOTAL: AtomicU32 = AtomicU32::new(1);

pub struct Psalm {
    pub day: u32,
    pub text: String,
}

// HOOK 1: scrape the psalms of one month. Call `progress(done, total)` as you go (drives the bar).
fn fetch(year: u32, month: u32, progress: &dyn Fn(u32, u32)) -> Result<Vec<Psalm>, String> {
    let _ = (year, month, progress);
    Err("pobieranie nie jest jeszcze zaimplementowane".into())
}

// HOOK 2: the chosen format's file contents.
fn render(psalms: &[Psalm], format: Format) -> Vec<u8> {
    let _ = psalms;
    match format {
        Format::Word2003 => vec![],
        Format::Tablica => vec![], // the app's own text format: see file_bytes() in file.rs
    }
}

// HOOK 3: put the file somewhere (file chooser + fs::write). Runs on the UI thread.
fn save(bytes: Vec<u8>, suggested_name: &str) -> Result<(), String> {
    let _ = (bytes, suggested_name);
    Err("zapis nie jest jeszcze zaimplementowany".into())
}

fn progress(done: u32, total: u32) {
    DONE.store(done, Relaxed);
    TOTAL.store(total, Relaxed);
}

fn start(year: u32, month: u32, status: &Frame) {
    if BUSY.swap(true, Relaxed) {
        return;
    }
    progress(0, 1);
    let mut st = status.clone();
    st.set_label_font(FONT); // monospace so the bar doesn't jiggle
    st.set_label(&bar(0, 1));
    let mut tick = st.clone();
    app::add_timeout3(0.1, move |h| {
        if BUSY.load(Relaxed) {
            tick.set_label(&bar(DONE.load(Relaxed), TOTAL.load(Relaxed)));
            app::repeat_timeout3(0.1, h);
        }
    });
    let (format, _, ext, _) = FORMATS[FORMAT.load(Relaxed)];
    bg(
        move || fetch(year, month, &progress).map(|p| render(&p, format)),
        move |r| {
            BUSY.store(false, Relaxed);
            st.set_label_font(Font::Helvetica);
            let name = format!("psalmy_{year}-{month:02}.{ext}");
            st.set_label(&match r.and_then(|b| save(b, &name)) {
                Ok(()) => "Zapisano".to_string(),
                Err(e) => format!("Błąd: {e}"),
            });
        },
    );
}

fn paint_marks(marks: &mut [Frame], t: &Theme) {
    let sel = FORMAT.load(Relaxed);
    for (i, m) in marks.iter_mut().enumerate() {
        let c = if i == sel {
            t.colors.accent
        } else {
            t.colors.panel
        };
        m.set_color(Color::from_hex(c));
    }
}

fn paint(marks: &mut [Frame], status: &mut Frame, panel: &mut Frame, t: &Theme) {
    paint_marks(marks, t);
    panel.set_color(Color::from_hex(t.colors.panel));
    status.set_color(Color::from_hex(t.colors.panel));
    status.set_label_color(Color::from_hex(t.colors.scrim));
}

// Create it hidden, inside the root group, like the other overlays.
pub fn psalms_overlay(ed: &text::TextEditor) -> group::Group {
    let (w, h) = (460, 348);
    let (ox, oy) = ((960 - w) / 2, (728 - h) / 2);
    let mut g = group::Group::new(0, 0, 960, 728, None);
    scrim();
    let mut panel = Frame::new(ox, oy, w, h, None);
    panel.set_frame(FrameType::BorderBox);
    let mut title = lbl("Psalmy na miesiąc", ox + 16, oy + 12, w - 32, 28, 20);
    title.set_label_font(Font::HelveticaBold);

    // year + month, default: next month
    let (y, m, _) = civil();
    let (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    lbl("Rok:", ox + 16, oy + 56, 40, 24, 14);
    let mut year = NumInput::new(ox + 60, oy + 56, 2000, 2100, false);
    year.set_value(y as u32);
    lbl("Miesiąc:", ox + 140, oy + 56, 70, 24, 14);
    let mut month = NumInput::new(ox + 214, oy + 56, 1, 12, false);
    month.set_value(m);

    // format: three 63x71 image buttons; the selected one sits on an accent-coloured mat
    lbl("Format:", ox + 16, oy + 96, 120, 24, 14);
    let mut marks: Vec<Frame> = (0..FORMATS.len() as i32)
        .map(|i| {
            let mut f = Frame::new(ox + 16 + i * 84, oy + 122, 71, 79, None);
            f.set_frame(FrameType::FlatBox);
            f
        })
        .collect();
    for (i, &(_, tex, _, caption)) in FORMATS.iter().enumerate() {
        let mut b = btn(tex, ox + 20 + i as i32 * 84, oy + 126);
        b.set_label(caption); // placeholder: drop these 3 lines once the textures carry their own text
        b.set_label_size(11);
        b.set_label_color(Color::from_hex(theme().colors.scrim));
        let mut marks = marks.clone();
        b.set_callback(move |b| {
            FORMAT.store(i, Relaxed);
            paint_marks(&mut marks, theme());
            if let Some(mut g) = b.parent() {
                g.redraw(); // the mats sit under the buttons
            }
        });
    }

    // download button: 428x44 (tex/pobierz_{off,on}.svg) + the progress bar / result line
    let mut dl = btn("pobierz", ox + 16, oy + 217);
    let mut status = lbl("", ox + 16, oy + 273, w - 32, 24, 13);
    status.set_frame(FrameType::FlatBox);
    dl.set_callback({
        let status = status.clone();
        move |_| start(year.value(), month.value(), &status)
    });

    let mut close = button::Button::new(ox + w - 116, oy + h - 44, 100, 28, "Zamknij");
    style_btn(&mut close);
    close.set_callback({
        let mut ed = ed.clone();
        move |b| {
            if let Some(mut g) = b.parent() {
                close_overlay(&mut g, &mut ed);
            }
        }
    });
    g.end();
    g.hide();
    modal_keys(&mut g, ed);

    paint(&mut marks, &mut status, &mut panel, theme());
    skin(move |t| paint(&mut marks, &mut status, &mut panel, t));
    g
}
