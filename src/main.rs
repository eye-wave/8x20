#![cfg_attr(windows, windows_subsystem = "windows")]
use encoding_rs::WINDOWS_1250;
use fltk::{dialog::*, enums::*, frame::Frame, prelude::*, *};
use std::{
    cell::{Cell, RefCell},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        LazyLock,
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc,
    },
};

mod engine;
mod numinput;
mod sign;
mod theme;

use engine::{Al, COLS, ROWS, align, align_line, bad, kill, put, rejoin, tidy, valign};
use numinput::NumInput;
use theme::{CUR, THEMES, Theme, apply_theme, img, pick_theme, png, seasonal_on, skin, theme};

pub static CONFIG_PATH: LazyLock<PathBuf> =
    LazyLock::new(|| dirs::config_dir().unwrap().join(".tablica_cfg"));

// Image button from <name>_off.svg + <name>_on.svg (on = hovered); the SVG's width/height set the size,
// so every theme's textures must have the same sizes.
fn btn(name: &'static str, x: i32, y: i32) -> button::Button {
    let t = theme();
    let imgs = Rc::new(RefCell::new((
        img(t, &format!("{name}_off")),
        img(t, &format!("{name}_on")),
    )));
    let (w, h) = (imgs.borrow().0.w(), imgs.borrow().0.h());
    let mut b = button::Button::new(x, y, w, h, None);
    b.set_frame(FrameType::NoBox);
    b.set_image(Some(imgs.borrow().0.clone()));
    b.handle({
        let imgs = imgs.clone();
        move |b, ev| match ev {
            Event::Enter | Event::Leave => {
                let i = imgs.borrow();
                b.set_image(Some(if ev == Event::Enter {
                    i.1.clone()
                } else {
                    i.0.clone()
                }));
                b.redraw();
                true
            }
            _ => false,
        }
    });
    skin({
        let mut b = b.clone();
        move |t| {
            let n = (
                img(t, &format!("{name}_off")),
                img(t, &format!("{name}_on")),
            );
            b.set_image(Some(n.0.clone()));
            *imgs.borrow_mut() = n;
            b.redraw();
        }
    });
    b
}

// Config = "key=value" lines. Parsing is split from file I/O so it can be tested.
fn parse_get(t: &str, k: &str) -> Option<String> {
    t.lines().find_map(|l| {
        let (key, v) = l.split_once('=')?;
        (key.trim() == k).then(|| v.trim().to_string())
    })
}

fn parse_set(t: &str, k: &str, v: &str) -> String {
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

// Decorative separator (sep.svg), vertically centred on cy.
fn sep(x: i32, cy: i32) {
    let i = img(theme(), "sep");
    let mut f = Frame::new(x, cy - i.h() / 2, i.w(), i.h(), None);
    f.set_image(Some(i));
    skin(move |t| {
        f.set_image(Some(img(t, "sep")));
        f.redraw();
    });
}

fn paint_dot(d: &mut Frame, t: &Theme, ok: bool) {
    d.set_image(Some(img(
        t,
        if ok {
            "status_dot_on"
        } else {
            "status_dot_off"
        },
    )));
    d.redraw();
}

fn default_title() -> String {
    align_line("TEKST NR 000", Al::Center)
}

fn lbl(text: &str, x: i32, y: i32, w: i32, h: i32, size: i32) -> Frame {
    let mut f = Frame::new(x, y, w, h, None);
    f.set_label(text);
    f.set_label_size(size);
    f.set_align(Align::Left | Align::Inside);
    f
}

// Counter labels ("K: 1   W: 1", "Strona - 001"): coloured by the theme's `page_info`.
fn page_info(f: Frame) -> Frame {
    let paint = |f: &mut Frame, t: &theme::Theme| {
        f.set_label_color(Color::from_hex(t.colors.page_info));
        f.redraw();
    };
    let mut c = f.clone();
    paint(&mut c, theme());
    skin(move |t| paint(&mut c, t));
    f
}

const FONT: Font = Font::Courier; // monospace
const SIZE: i32 = 30;

// Editor box is exactly COLS x ROWS character cells (fltk text margins: 3px sides, 1px top/bottom)
// plus a vertical scrollbar of `bar` px (1 = effectively none).
fn editor(
    x: i32,
    y: i32,
    bar: i32,
    slack: i32,
    show: Rc<Cell<bool>>,
    pages: bool, // draw page separators
) -> (text::TextEditor, text::TextBuffer) {
    draw::set_font(FONT, SIZE);
    let w = COLS as i32 * draw::width("M").round() as i32 + 6 + bar;
    let h = ROWS * draw::height() + 2 + slack; // slack: spare px so rounding never makes the last row scroll
    let buf = text::TextBuffer::default();
    let mut ed = text::TextEditor::new(x, y, w, h, None);
    ed.set_buffer(buf.clone());
    ed.set_frame(FrameType::FlatBox);
    ed.set_text_font(FONT);
    ed.set_text_size(SIZE);
    let colour = |e: &mut text::TextEditor, t: &theme::Theme| {
        e.set_color(Color::from_hex(t.colors.text_bg));
        e.set_text_color(Color::from_hex(t.colors.text));
        e.set_cursor_color(Color::from_hex(t.colors.text)); // the caret follows the text colour
        e.redraw();
    };
    colour(&mut ed, theme());
    skin({
        let mut e = ed.clone();
        move |t| colour(&mut e, t)
    });
    ed.set_scrollbar_size(bar);
    ed.set_scrollbar_align(Align::Right); // vertical bar only: fltk shows the horizontal one only if TOP/BOTTOM is set
    // Tab: not handled by the editor, so fltk moves focus to the next widget instead of inserting.
    ed.add_key_binding(Key::Tab, Shortcut::None, |_, _| 0);
    // Ctrl+Backspace / Ctrl+Delete: delete a word (fltk's default treats them as plain Backspace/Delete).
    // fltk wants plain fn pointers here, so rebuild the editor from the raw pointer (never dropped).
    ed.add_key_binding(Key::BackSpace, Shortcut::Ctrl, |_, p| {
        let mut e =
            std::mem::ManuallyDrop::new(unsafe { text::TextEditor::from_widget_ptr(p as _) });
        kill(&mut e, true)
    });
    ed.add_key_binding(Key::Delete, Shortcut::Ctrl, |_, p| {
        let mut e =
            std::mem::ManuallyDrop::new(unsafe { text::TextEditor::from_widget_ptr(p as _) });
        kill(&mut e, false)
    });
    // Overlay after the base draw: a line above every page's first row, and (optionally) · / ¶ for
    // invisible characters, which fltk can't show.
    ed.super_draw_first(true);
    ed.super_handle_first(false); // our handler (right-click menu, drops) must run before FLTK's own
    ed.draw(move |e| {
        let Some(b) = e.buffer() else {
            return;
        };
        let from = e.xy_to_position(e.x(), e.y(), text::PositionType::Cursor);
        draw::push_clip(e.x(), e.y(), e.w(), e.h());
        draw::set_font(FONT, SIZE);
        let (cw, lh) = (draw::width("M").round() as i32, draw::height());
        let (n0, y0) = (
            b.count_lines(0, from),
            e.position_to_xy(b.line_start(from)).1,
        );
        draw::set_draw_color(Color::from_hex(theme().colors.page_line));
        for r in 0..=e.h() / lh + 1 {
            if pages && (n0 + r) % ROWS == 0 && n0 + r > 0 {
                draw::draw_line(e.x(), y0 + r * lh, e.x() + e.w() - 12, y0 + r * lh);
            }
        }
        if show.get() {
            let to = e.xy_to_position(e.x() + e.w(), e.y() + e.h(), text::PositionType::Cursor);
            draw::set_draw_color(Color::from_hex(theme().colors.marks));
            for (i, ch) in b.text().char_indices() {
                let i = i as i32;
                let g = match ch {
                    _ if i < from || i > to => continue,
                    ' ' => "·",
                    '\n' => "¶",
                    _ => continue,
                };
                let (px, py) = e.position_to_xy(i);
                draw::draw_text2(g, px, py, cw, lh, Align::Center);
            }
        }
        draw::pop_clip();
    });
    (ed, buf)
}

type OnDrop = Rc<dyn Fn(PathBuf)>;
thread_local!(static ON_DROP: RefCell<Option<OnDrop>> = const { RefCell::new(None) });
static DROP: AtomicBool = AtomicBool::new(false);

// "file:///a%20b" (X11/Wayland) or a plain path (Windows) -> path
fn uri_path(l: &str) -> PathBuf {
    let l = l.trim();
    let Some(r) = l.strip_prefix("file://") else {
        return l.into();
    };
    let (b, mut v, mut i) = (r.as_bytes(), vec![], 0);
    while i < b.len() {
        match r
            .get(i + 1..i + 3)
            .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            Some(x) if b[i] == b'%' => (v.push(x), i += 3),
            _ => (v.push(b[i]), i += 1),
        };
    }
    String::from_utf8_lossy(&v).into_owned().into()
}

// Drag & drop for the window and every widget: never pastes dropped paths into a text area; a dropped
// file is opened instead. FLTK delivers the drop as Paste right after DndRelease.
pub fn dnd(ev: Event) -> bool {
    match ev {
        Event::DndEnter | Event::DndLeave => {
            DROP.store(false, Relaxed);
            true
        }
        Event::DndDrag => true,
        Event::DndRelease => {
            DROP.store(true, Relaxed);
            true
        }
        Event::Paste if DROP.swap(false, Relaxed) => {
            let p = uri_path(app::event_text().lines().next().unwrap_or_default());
            if p.is_file()
                && !MODAL.load(Relaxed)
                && let Some(f) = ON_DROP.with(|h| h.borrow().clone())
            {
                f(p)
            }
            true
        }
        _ => false,
    }
}

// Polish right-click menu for the editors (FLTK has none for text editors).
fn ctx(e: &mut text::TextEditor) {
    let Some(mut b) = e.buffer() else { return };
    let _ = e.take_focus();
    let m = menu::MenuItem::new(&["Wytnij", "Kopiuj", "Wklej", "Zaznacz wszystko"])
        .popup(app::event_x(), app::event_y());
    let sel = b.selection_text();
    match m.and_then(|m| m.label()).as_deref() {
        Some("Wytnij") if !sel.is_empty() => {
            app::copy(&sel);
            b.remove_selection();
        }
        Some("Kopiuj") if !sel.is_empty() => app::copy(&sel),
        Some("Wklej") => app::paste(&*e),
        Some("Zaznacz wszystko") => b.select(0, b.length()),
        _ => {}
    }
}

// Load a file into the editors and remember it as the current file.
fn open_file(
    f: &Path,
    b: &mut text::TextBuffer,
    tb: &mut text::TextBuffer,
    path: &RefCell<Option<PathBuf>>,
    msg: &mut Frame,
) {
    match fs::read(f)
        .map_err(|e| e.to_string())
        .and_then(|t| load(&t))
    {
        Ok(l) => {
            tb.set_text(&l.title);
            b.set_text(&l.body);
            *path.borrow_mut() = Some(f.to_path_buf());
            msg.set_label(&if l.soft + l.hard > 0 {
                format!(
                    "Otwarto; zawinięto linie: miękko {}, twardo {}",
                    l.soft, l.hard
                )
            } else {
                "Otwarto".to_string()
            });
        }
        Err(e) => alert_default(&format!("Nie można otworzyć: {e}")),
    }
}

const DECO: &[u8] = b"<------------------>";

// File format: every page (title page = 000 first) is DECO / "<- Strona nr: NNN ->" / DECO
// followed by exactly ROWS rows of at most COLS bytes (Windows-1250, CRLF).
fn file_bytes(pages: &sign::Pages) -> Vec<u8> {
    let mut out = vec![];
    for (k, rows) in pages.iter().enumerate() {
        out.extend_from_slice(DECO);
        out.extend_from_slice(format!("\r\n<- Strona nr: {k:03} ->\r\n").as_bytes());
        out.extend_from_slice(DECO);
        out.extend_from_slice(b"\r\n");
        for i in 0..ROWS as usize {
            out.extend_from_slice(rows.get(i).map_or(&[][..], |r| &r[..]));
            out.extend_from_slice(b"\r\n");
        }
    }
    out
}

// A decoration line such as <------------------> (only '<', '-' and '>' characters, at least one arrow).
fn is_deco(l: &str) -> bool {
    let t = l.trim();
    t.contains(['<', '>']) && t.chars().all(|c| matches!(c, '<' | '-' | '>'))
}

// "<- Strona nr: 007 ->" -> Some(7); tolerant of spacing and of a missing closing arrow.
fn page_header(l: &str) -> Option<usize> {
    let d = l.trim().strip_prefix("<- Strona nr:")?.trim_start();
    d.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

// Fit a line into COLS: soft wrap at the last space that fits (indent kept, the spaces at the break
// dropped); a word longer than a line is hard-split at COLS. Tabs become spaces, other control
// characters are dropped, trailing spaces are layout and removed.
fn wrap_line(l: &str, soft: &mut usize, hard: &mut usize) -> Vec<String> {
    let mut c: Vec<char> = l
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    while c.last() == Some(&' ') {
        c.pop();
    }
    let mut out = vec![];
    while c.len() > COLS {
        let lead = c.iter().take_while(|&&x| x == ' ').count();
        match (lead + 1..=COLS).rev().find(|&i| c[i] == ' ') {
            Some(i) => {
                out.push(c[..i].iter().collect::<String>().trim_end().to_string());
                let k = i + c[i..].iter().take_while(|&&x| x == ' ').count();
                c.drain(..k);
                *soft += 1;
            }
            None => {
                out.push(c[..COLS].iter().collect());
                c.drain(..COLS);
                *hard += 1;
            }
        }
    }
    out.push(c.into_iter().collect());
    out
}

struct Loaded {
    title: String,
    body: String,
    soft: usize, // lines that were soft wrapped
    hard: usize, // lines that were hard wrapped
}

// Load a text file with or without the page decoration (DECO / "<- Strona nr: NNN ->" / DECO):
// - decoration lines are optional and ignored; a "<- Strona nr: N ->" line alone starts page N;
// - rows before the first header belong to the title page (page 0);
// - a file with no header at all is plain text: all of it is the body, the title stays default;
// - UTF-8 is accepted as well as Windows-1250;
// - lines over COLS are soft/hard wrapped; a page that grows past ROWS rows spills onto an extra
//   page (the following pages move down so no text is lost or merged).
fn load(bytes: &[u8]) -> Result<Loaded, String> {
    let text = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => WINDOWS_1250.decode(bytes).0.into_owned(),
    };
    let text = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last().is_some_and(|x| x.is_empty()) {
        lines.pop();
    }
    let (mut soft, mut hard) = (0, 0);
    let mut pages: Vec<Vec<String>> = vec![vec![]];
    let (mut cur, mut headed) = (0usize, false);
    for l in lines {
        if is_deco(l) {
            continue;
        }
        if let Some(n) = page_header(l) {
            if n > 127 {
                return Err(format!("Strona {n} za wysoka (max 127)"));
            }
            (cur, headed) = (n, true);
            if pages.len() <= n {
                pages.resize(n + 1, vec![]);
            }
            continue;
        }
        pages[cur].extend(wrap_line(l, &mut soft, &mut hard));
    }
    let title = if headed {
        let keep = pages[0].len().min(ROWS as usize);
        let extra = pages[0].split_off(keep);
        if !extra.is_empty() {
            pages.insert(1, extra); // title rows beyond ROWS get a body page of their own
        }
        let mut t = pages[0].clone();
        while t.last().is_some_and(|x| x.is_empty()) {
            t.pop();
        }
        t.join("\n")
    } else {
        let all = std::mem::take(&mut pages[0]);
        pages.push(all);
        default_title()
    };
    let mut rows: Vec<String> = vec![];
    for p in &pages[1..] {
        let n = p.len().div_ceil(ROWS as usize).max(1) * ROWS as usize;
        rows.extend(p.iter().cloned());
        rows.resize(rows.len() + n - p.len(), String::new()); // pad to whole pages
    }
    while rows.last().is_some_and(|x| x.is_empty()) {
        rows.pop();
    }
    Ok(Loaded {
        title,
        body: rows.join("\n"),
        soft,
        hard,
    })
}

// Enforces the line rules on the buffer itself (so it works however fltk orders its key handling):
// any edit that breaks them is taken back on the next loop tick; pastes are re-inserted hard-wrapped.
// Also keeps K (column) / W (row) and the page label fresh.
fn rules(
    ed: &mut text::TextEditor,
    buf: &text::TextBuffer,
    max_lines: Option<i32>,
    mut kw: Frame,
    mut page: Option<Frame>,
) {
    let mut watch = buf.clone();
    watch.add_modify_callback({
        let (b, e) = (buf.clone(), ed.clone());
        move |pos, ins, del, _r, t| {
            if ins == 0 && t == "\n" {
                let (mut b, mut e) = (b.clone(), e.clone());
                app::add_timeout3(0.0, move |_| rejoin(&mut e, &mut b, pos));
            } else if ins > 0 && bad(&b, pos, ins, max_lines) {
                let (mut b, mut e) = (b.clone(), e.clone());
                app::add_timeout3(0.0, move |_| {
                    let txt = b
                        .text_range(pos, (pos + ins).min(b.length()))
                        .unwrap_or_default();
                    b.remove(pos, (pos + ins).min(b.length()));
                    e.set_insert_position(pos);
                    put(&mut e, &mut b, &txt, max_lines, ins > 1);
                });
            }
            if ins > 0 || del > 0 {
                let (mut b, e) = (b.clone(), e.clone());
                app::add_timeout3(0.0, move |_| tidy(&e, &mut b));
            }
        }
    });
    let b = buf.clone();
    let mut upd = move |e: &text::TextEditor| {
        let p = e.insert_position();
        let row = b.count_lines(0, p) + 1;
        let col = b
            .text_range(b.line_start(p), p)
            .unwrap_or_default()
            .chars()
            .count()
            + 1;
        kw.set_label(&format!("K: {col}   W: {row}"));
        if let Some(pg) = page.as_mut() {
            pg.set_label(&format!("Strona - {:03}", (row - 1) / ROWS + 1));
        }
    };
    ed.handle(move |e, ev| {
        if dnd(ev) {
            return true;
        }
        if ev == Event::Push && app::event_mouse_button() == app::MouseButton::Right {
            ctx(e);
            return true;
        }
        if matches!(ev, Event::KeyUp | Event::Released | Event::Focus) {
            upd(e);
        }
        false
    });
}

// "[#####               ] 25%"
fn bar(done: u32, total: u32) -> String {
    let p = (done * 100 / total.max(1)).min(100);
    let n = (p / 5) as usize;
    format!("[{}{}] {p}%", "#".repeat(n), " ".repeat(20 - n))
}

// Selected port name (index 0 is the "not selected" entry). The names are kept separately because
// FLTK's menu labels treat '/' as a submenu separator.
fn port_of(c: &menu::Choice, names: &[String]) -> Option<String> {
    usize::try_from(c.value() - 1)
        .ok()
        .and_then(|i| names.get(i).cloned())
}

// Digits-aware comparison: COM3 < COM10, ttyS3 < ttyS10; case-insensitive.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::{cmp::Ordering::*, iter::Peekable, str::Chars};
    fn num(it: &mut Peekable<Chars>) -> String {
        let mut s = String::new();
        while let Some(&c) = it.peek().filter(|c| c.is_ascii_digit()) {
            s.push(c);
            it.next();
        }
        s
    }
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Equal,
            (None, _) => return Less,
            (_, None) => return Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (na, nb) = (num(&mut a), num(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                match ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)) {
                    Equal => {}
                    o => return o,
                }
            }
            (Some(x), Some(y)) => {
                match x.to_lowercase().cmp(y.to_lowercase()) {
                    Equal => {}
                    o => return o,
                }
                a.next();
                b.next();
            }
        }
    }
}

// Run a blocking job on a thread; deliver its result on the UI thread.
fn bg<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(job());
    });
    let mut done = Some(done);
    app::add_timeout3(0.1, move |h| match rx.try_recv() {
        Ok(v) => {
            if let Some(d) = done.take() {
                d(v)
            }
        }
        Err(mpsc::TryRecvError::Empty) => app::repeat_timeout3(0.1, h),
        Err(_) => {}
    });
}

// Sign rows (Windows-1250, space padded) -> editor text; trailing blank rows dropped.
fn decode(rows: &[Vec<u8>]) -> String {
    let mut v: Vec<String> = rows
        .iter()
        .map(|r| {
            WINDOWS_1250
                .decode(r)
                .0
                .trim_end_matches([' ', '\0'])
                .to_string()
        })
        .collect();
    while v.last().is_some_and(|l| l.is_empty()) {
        v.pop();
    }
    v.join("\n")
}

// Editor text (UTF-8) -> Windows-1250 bytes. A character the code page lacks becomes '?' (counted in
// `replaced`); it is never an error. Zero-width characters are invisible, so they are just dropped.
fn cp1250(l: &str, replaced: &mut usize) -> Vec<u8> {
    l.chars()
        .filter(|c| !matches!(c, '\u{200b}'..='\u{200d}' | '\u{feff}'))
        .map(|c| {
            let mut buf = [0u8; 4];
            let (b, _, bad) = WINDOWS_1250.encode(c.encode_utf8(&mut buf));
            if bad {
                *replaced += 1;
                b'?'
            } else {
                b[0]
            }
        })
        .collect()
}

// Editor text -> pages (title page first, then the body in chunks of ROWS lines) as Windows-1250,
// plus how many characters had to be replaced. Used by both save and send.
fn to_pages(title: &str, body: &str) -> Result<(sign::Pages, usize), String> {
    let mut replaced = 0;
    let mut rows = |t: &str| -> Result<Vec<Vec<u8>>, String> {
        t.lines()
            .map(|l| {
                let b = cp1250(l, &mut replaced);
                if b.len() > COLS {
                    Err(format!("Linia ma więcej niż {COLS} znaków: {l}"))
                } else {
                    Ok(b)
                }
            })
            .collect()
    };
    let mut pages = vec![rows(title)?];
    if pages[0].len() > ROWS as usize {
        return Err(format!("Strona tytułowa ma więcej niż {ROWS} linii"));
    }
    pages.extend(rows(body)?.chunks(ROWS as usize).map(|c| c.to_vec()));
    if pages.len() > 128 {
        return Err("Za dużo stron (max 127)".into());
    }
    Ok((pages, replaced))
}

// While the "Więcej" overlay is up, dropped files are ignored too.
static MODAL: AtomicBool = AtomicBool::new(false);

fn close_overlay(g: &mut group::Group, ed: &mut text::TextEditor) {
    g.hide();
    MODAL.store(false, Relaxed);
    let _ = ed.take_focus();
}

// "Więcej" overlay, drawn on top of the whole UI (create it last, inside the root group). It dims what
// is behind it and swallows every mouse/key event, so nothing underneath can be touched.
// UI only for now: none of the controls do anything yet.
fn more_overlay(ed: &text::TextEditor) -> group::Group {
    // Theme thumbnails sit in a grid of 3 columns; each cell is a 128x100 thumbnail + a 44px wrapped
    // label (row pitch 156). At most 2 rows are shown, further rows scroll, so the dialog never
    // outgrows the window. `dy` = how much taller the dialog is than with the old single thumbnail row.
    let rows = THEMES.len().div_ceil(3) as i32;
    let vh = rows.min(2) * 156 - 8;
    let dy = vh - 100;
    let (ox, oy) = ((960 - 460) / 2, (728 - (440 + dy)) / 2);
    let mut g = group::Group::new(0, 0, 960, 728, None);

    // dim the app behind with a 50% stipple (FLTK has no alpha fill)
    let mut scrim = Frame::new(0, 0, 960, 728, None);
    scrim.draw(|f| {
        draw::set_draw_color(Color::from_hex(theme().colors.scrim));
        for y in (f.y()..f.y() + f.h()).step_by(2) {
            draw::draw_line(f.x(), y, f.x() + f.w(), y);
        }
    });

    let mut panel = Frame::new(ox, oy, 460, 440 + dy, None);
    panel.set_frame(FrameType::BorderBox);
    panel.set_color(Color::from_hex(theme().colors.panel));

    let mut title = lbl("Dodatki", ox + 16, oy + 12, 428, 28, 20);
    title.set_label_font(Font::HelveticaBold);

    let mut psalms = button::Button::new(ox + 16, oy + 52, 428, 36, "Psalmy na miesiąc");
    psalms.set_tooltip("Pobieranie Psalmów na miesiąc");

    let mut themes = button::CheckButton::new(ox + 16, oy + 104, 428, 24, "Motywy okolicznościowe");
    themes.set_value(seasonal_on());

    // theme thumbnails (radio: one selected) with their names; clicking one applies and saves it
    let mut sc = group::Scroll::new(ox + 16, oy + 136, 428, vh, None);
    sc.set_type(group::ScrollType::Vertical);
    sc.set_scrollbar_size(12);
    sc.set_frame(FrameType::FlatBox);
    sc.set_color(Color::from_hex(theme().colors.panel));
    let mut radios = vec![];
    for (i, (key, t)) in THEMES.iter().enumerate() {
        let (cx, cy) = (
            ox + 16 + 136 * (i % 3) as i32,
            oy + 136 + 156 * (i / 3) as i32,
        );
        let mut thumb = png("bg", t.bg);
        thumb.scale(120, 91, true, true);
        let mut b = button::RadioButton::new(cx, cy, 128, 100, None);
        b.set_frame(FrameType::BorderBox);
        b.set_down_frame(FrameType::BorderBox);
        b.set_color(Color::White);
        b.set_selection_color(Color::from_hex(theme().colors.accent));
        b.set_image(Some(thumb));
        b.set_tooltip(t.name);
        b.set_value(i == CUR.get());
        b.set_callback(move |_| {
            apply_theme(i);
            cfg_set("theme", key);
        });
        radios.push(b);
        let mut name = Frame::new(cx, cy + 104, 128, 44, None);
        name.set_label(t.name);
        name.set_label_size(12);
        name.set_align(Align::Top | Align::Inside | Align::Wrap | Align::Clip); // wraps, never spills out
    }
    sc.end();
    themes.set_callback({
        let mut radios = radios.clone();
        move |c| {
            cfg_set("seasonal", if c.value() { "1" } else { "0" });
            apply_theme(pick_theme());
            for (i, r) in radios.iter_mut().enumerate() {
                r.set_value(i == CUR.get());
            }
        }
    });

    let mut line = Frame::new(ox + 16, oy + 248 + dy, 428, 1, None);
    line.set_frame(FrameType::FlatBox);
    line.set_color(Color::from_hex(theme().colors.rule));
    lbl("Przetwarzanie wsadowe", ox + 16, oy + 256 + dy, 428, 24, 14);
    // batch read / write / interrupt: the same textures as the toolbar (the main interrupt is under the overlay)
    btn("odczytaj", ox + 16, oy + 284 + dy);
    btn("wyslij", ox + 88, oy + 284 + dy);
    btn("przerwij", ox + 160, oy + 284 + dy);
    let mut prog = lbl(&bar(0, 1), ox + 16, oy + 356 + dy, 428, 24, 13);
    prog.set_label_font(FONT);
    skin({
        let (mut radios, mut sc) = (radios.clone(), sc.clone());
        move |t| {
            panel.set_color(Color::from_hex(t.colors.panel));
            sc.set_color(Color::from_hex(t.colors.panel));
            sc.redraw();
            line.set_color(Color::from_hex(t.colors.rule));
            for r in radios.iter_mut() {
                r.set_selection_color(Color::from_hex(t.colors.accent));
            }
            panel.redraw();
        }
    });

    let mut close = button::Button::new(ox + 344, oy + 398 + dy, 100, 28, "Zamknij");
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
    // Esc closes; every other mouse/key event is swallowed (the group's own handler runs after its children)
    g.handle({
        let mut ed = ed.clone();
        move |g, ev| {
            if ev == Event::KeyDown && app::event_key() == Key::Escape {
                close_overlay(g, &mut ed);
                return true;
            }
            matches!(
                ev,
                Event::Push
                    | Event::Released
                    | Event::Drag
                    | Event::Move
                    | Event::MouseWheel
                    | Event::KeyDown
                    | Event::KeyUp
                    | Event::Shortcut
            )
        }
    });
    g
}

fn main() {
    let app = app::App::default();
    // the UI is a fixed 960x728; on a smaller screen scale it down to fit the work area
    let (_, _, ww, wh) = app::screen_work_area(0);
    let k = (ww as f32 / 960.0).min(wh as f32 / 728.0);
    if k < 1.0 {
        app::set_screen_scale(0, k);
    }
    app::set_visible_focus(false);
    app::set_font_size(14);

    CUR.set(pick_theme()); // seasonal / saved theme, before any widget is built
    let mut win = window::Window::default()
        .with_size(960, 728)
        .with_label("Tablica");
    win.set_border(false);
    win.set_icon(Some(png("icon", theme::ICON)));
    win.size_range(960, 728, 0, 0); // resizable by the window manager (needed for maximize)

    // All content lives in one fixed-size group that is centered when the window grows.
    let mut root = group::Group::new(0, 0, 960, 728, None);

    // backdrop: title bar, toolbar/page panels, captions and editor frames are baked into one texture
    let mut backdrop = Frame::new(0, 0, 960, 728, None);
    backdrop.set_image(Some(png("bg", theme().bg)));
    skin({
        let mut b = backdrop.clone();
        move |t| {
            b.set_image(Some(png("bg", t.bg)));
            b.redraw();
        }
    });

    // drag the borderless window by its title bar
    let mut top = Frame::new(0, 0, 960, 66, None);
    let off = Rc::new(Cell::new((0, 0)));
    top.handle({
        let (mut w, off) = (win.clone(), off.clone());
        move |_, ev| match ev {
            Event::Push if !w.maximize_active() => {
                off.set((app::event_x_root() - w.x(), app::event_y_root() - w.y()));
                true
            }
            Event::Drag if !w.maximize_active() => {
                let (ox, oy) = off.get();
                w.set_pos(app::event_x_root() - ox, app::event_y_root() - oy);
                true
            }
            _ => false,
        }
    });

    // window buttons (the button SVGs are baked onto bg.png at exactly these positions)
    let mut min = btn("min", 882, 12);
    let mut close = btn("close", 928, 12);
    min.set_callback({
        let mut w = win.clone();
        move |_| w.iconize()
    });
    close.set_callback(|_| app::quit());

    // toolbar
    let mut new = btn("nowy", 19, 82);
    let mut open = btn("otworz", 87, 82);
    let mut save = btn("zapisz", 155, 82);
    let mut get = btn("odczytaj", 223, 82);
    let mut put_sign = btn("wyslij", 291, 82);
    // interrupt button: right after wyslij (must be made before root.end())
    let mut stop = btn("przerwij", 359, 82);
    // slot number above read, page count above write (centred over the 64px buttons)
    let mut num = NumInput::new(229, 56, 0, 999, false);
    num.set_value(1);
    let pgs = NumInput::new(297, 56, 0, 127, true); // 0 = auto
    let mut more = btn("dodatki", 814, 82); // "Więcej";
    let mut info = btn("info", 882, 82);
    // Groups left to right: files (64px) | horizontal align (36px each, 4px apart) |
    // vertical align (top / centred) | invisibles | settings. The groups after the files use
    // 12px gutters around a 2px separator because the toolbar is tight.
    let (mut left, mut center, mut right, mut just) = (
        btn("lewo", 449, 100),
        btn("srodek", 489, 100),
        btn("prawo", 529, 100),
        btn("justuj", 569, 100),
    );
    let (mut vtop, mut vmid) = (btn("pion_gora", 631, 100), btn("pion_srodek", 671, 100));
    let mut marks = btn("znaki", 733, 100);
    for x in [435, 617, 719, 793] {
        sep(x, 118);
    }

    const INFO: &str = concat!(
        "Tablica świetlna 8x20\n",
        "Wersja aplikacji: v",
        env!("CARGO_PKG_VERSION")
    );

    info.set_callback(|_| message_default(INFO));

    let show = Rc::new(Cell::new(false));

    // left panel: title page
    let kw1 = page_info(lbl("K: 1   W: 1", 290, 183, 120, 34, 14));
    let (mut title_ed, mut title_buf) = editor(36, 227, 1, 8, show.clone(), false);
    rules(&mut title_ed, &title_buf, Some(ROWS), kw1, None);
    title_buf.set_text(&default_title());

    // right panel: page
    let page = page_info(lbl("Strona - 001", 586, 183, 200, 34, 17));
    let kw2 = page_info(lbl("K: 1   W: 1", 804, 183, 120, 34, 14));
    let mut search = btn("szukaj", 898, 186);
    let (mut ed, buf) = editor(550, 227, 12, 48, show.clone(), true);
    rules(&mut ed, &buf, None, kw2, Some(page));

    // status bar
    let mut com = lbl("COM-", 60, 676, 100, 24, 16);
    com.set_label_color(Color::from_hex(theme().colors.label));
    let mut msg = lbl("", 300, 670, 345, 24, 13); // the status widget in the middle of the bar
    msg.set_align(Align::Center | Align::Inside);
    msg.set_label_color(Color::from_hex(theme().colors.label)); // same as the COM label
    skin({
        let (mut c, mut m) = (com.clone(), msg.clone());
        move |t| {
            for w in [&mut c, &mut m] {
                w.set_label_color(Color::from_hex(t.colors.label));
                w.redraw();
            }
        }
    });
    // connection status dot, left of the port dropdown: green after a successful read/write,
    // red after a failure and whenever another port is picked
    let ok = Rc::new(Cell::new(false));
    let mut dot = Frame::new(709, 679, 15, 15, None);
    paint_dot(&mut dot, theme(), false);
    skin({
        let (mut d, ok) = (dot.clone(), ok.clone());
        move |t| paint_dot(&mut d, t, ok.get())
    });
    let set_dot: Rc<dyn Fn(bool)> = Rc::new(move |v| {
        ok.set(v);
        paint_dot(&mut dot.clone(), theme(), v);
    });
    let mut names = sign::ports();
    names.sort_by(|a, b| natural_cmp(a, b).then_with(|| a.cmp(b)));
    let ports = Rc::new(names);
    let mut port = menu::Choice::new(730, 674, 210, 26, None);
    port.add_choice("Port szeregowy niewybrany");
    for name in ports.iter() {
        port.add_choice(&name.replace('/', "\\/")); // a bare '/' would start a submenu
    }
    port.set_value(0);
    port.set_text_size(13);
    port.set_callback({
        let (mut com, set_dot, ports) = (com.clone(), set_dot.clone(), ports.clone());
        move |p| {
            com.set_label(&port_of(p, &ports).unwrap_or_else(|| "COM-".into()));
            set_dot(false);
        }
    });

    // reselect the port that last worked (if it is still plugged in)
    if let Some(i) = cfg_get("port").and_then(|s| ports.iter().position(|p| *p == s)) {
        port.set_value(i as i32 + 1);
        com.set_label(&ports[i]);
    }

    // "Więcej" overlay: last child of root, so it sits on top of everything
    let overlay = more_overlay(&ed);
    more.set_callback({
        let mut o = overlay.clone();
        move |_| {
            MODAL.store(true, Relaxed);
            o.show();
            app::set_focus(&o); // keys must not reach the editor underneath
        }
    });

    root.end();
    win.end();
    win.resize_callback(move |_, _, _, w, h| root.set_pos((w - 960) / 2, (h - 728) / 2));
    win.super_handle_first(false);
    win.handle(|_, ev| dnd(ev));
    win.show();
    let _ = ed.take_focus(); // start typing in the page editor

    // alignment acts on the focused editor (body if none); clicks don't steal focus (visible_focus is off)
    let go: Rc<dyn Fn(Al)> = Rc::new({
        let (te, tb, e, b) = (title_ed.clone(), title_buf.clone(), ed.clone(), buf.clone());
        move |a| {
            let title = app::focus().is_some_and(|f| f.is_same(&te));
            let (mut e, mut b) = if title {
                (te.clone(), tb.clone())
            } else {
                (e.clone(), b.clone())
            };
            align(&mut e, &mut b, a);
        }
    });
    for (btn, a) in [
        (&mut left, Al::Left),
        (&mut center, Al::Center),
        (&mut right, Al::Right),
        (&mut just, Al::Justify),
    ] {
        let go = go.clone();
        btn.set_callback(move |_| go(a));
    }
    // vertical alignment: the current page, or every page the selection touches
    let vgo: Rc<dyn Fn(bool)> = Rc::new({
        let (te, tb, e, b) = (title_ed.clone(), title_buf.clone(), ed.clone(), buf.clone());
        move |mid| {
            let title = app::focus().is_some_and(|f| f.is_same(&te));
            let (mut e, mut b) = if title {
                (te.clone(), tb.clone())
            } else {
                (e.clone(), b.clone())
            };
            valign(&mut e, &mut b, mid);
        }
    });
    for (btn, mid) in [(&mut vtop, false), (&mut vmid, true)] {
        let vgo = vgo.clone();
        btn.set_callback(move |_| vgo(mid));
    }
    marks.set_callback({
        let (show, mut te, mut e) = (show.clone(), title_ed.clone(), ed.clone());
        move |_| {
            show.set(!show.get());
            te.redraw();
            e.redraw();
        }
    });

    // actions (plain text, right-hand page)
    let path: Rc<RefCell<Option<PathBuf>>> = Rc::default();
    new.set_callback({
        let (mut b, mut tb, path) = (buf.clone(), title_buf.clone(), path.clone());
        move |_| {
            // a project that is empty (or still the untouched default) is replaced without asking
            let dirty = !b.text().is_empty() || tb.text() != default_title();
            if dirty
                && choice2_default(
                    "Bieżący tekst zostanie usunięty. Utworzyć nowy?",
                    "Nie",
                    "Tak",
                    "",
                ) != Some(1)
            {
                return;
            }
            b.set_text("");
            tb.set_text(&default_title());
            *path.borrow_mut() = None;
        }
    });
    let open_path: OnDrop = Rc::new({
        let (b, tb, path, msg) = (buf.clone(), title_buf.clone(), path.clone(), msg.clone());
        move |f| open_file(&f, &mut b.clone(), &mut tb.clone(), &path, &mut msg.clone())
    });
    ON_DROP.with(|h| *h.borrow_mut() = Some(open_path.clone()));
    open.set_callback({
        let open_path = open_path.clone();
        move |_| {
            let mut d = NativeFileChooser::new(NativeFileChooserType::BrowseFile);
            d.show();
            let f = d.filename();
            if !f.as_os_str().is_empty() {
                open_path(f);
            }
        }
    });
    // "Open with" / file argument: load it at launch
    if let Some(a) = std::env::args_os().nth(1) {
        open_path(a.into());
    }
    save.set_callback({
        let (b, tb, path, mut msg) = (buf.clone(), title_buf.clone(), path.clone(), msg.clone());
        move |_| {
            let (pages, rep) = match to_pages(&tb.text(), &b.text()) {
                Ok(r) => r,
                Err(e) => return alert_default(&format!("Nie można zapisać: {e}")),
            };
            let bytes = file_bytes(&pages);
            if path.borrow().is_none() {
                let mut d = NativeFileChooser::new(NativeFileChooserType::BrowseSaveFile);
                d.set_option(NativeFileChooserOptions::SaveAsConfirm);
                d.show();
                let f = d.filename();
                if f.as_os_str().is_empty() {
                    return;
                }
                *path.borrow_mut() = Some(f);
            }
            let p = path.borrow().clone().unwrap();
            match fs::write(&p, bytes) {
                Ok(()) => msg.set_label(&if rep > 0 {
                    format!("Zapisano (Windows-1250); zamieniono znaki: {rep}")
                } else {
                    "Zapisano".to_string()
                }),
                Err(e) => alert_default(&format!("Nie można zapisać: {e}")),
            }
        }
    });
    search.set_callback({
        let (mut ed, mut b) = (ed.clone(), buf.clone());
        move |_| {
            if let Some(q) = input_default("Szukaj:", "")
                && let Some(p) = b.search_forward(ed.insert_position(), &q, false)
            {
                let e = p + q.len() as i32;
                b.select(p, e);
                ed.set_insert_position(e);
                ed.show_insert_position();
            }
        }
    });

    // projector read / write (slow serial I/O runs on a worker thread)
    let busy = Rc::new(Cell::new(false));
    stop.set_callback({
        let (busy, mut msg) = (busy.clone(), msg.clone());
        move |_| {
            if busy.get() {
                sign::cancel();
                msg.set_label("Przerywanie...");
            }
        }
    });
    get.set_callback({
        let (port, ports, num, pgs, mut msg, buf, tbuf, busy, set_dot) = (
            port.clone(),
            ports.clone(),
            num.clone(),
            pgs.clone(),
            msg.clone(),
            buf.clone(),
            title_buf.clone(),
            busy.clone(),
            set_dot.clone(),
        );
        move |_| {
            if busy.get() {
                return;
            }
            let Some(name) = port_of(&port, &ports) else {
                set_dot(false);
                msg.set_label("Wybierz port szeregowy");
                return;
            };
            let (n, k) = (num.value() as u16, pgs.value() as u8);
            busy.set(true);
            // monospace so the bar doesn't jiggle; auto mode (0 pages) counts against the 127-page max
            let total = 1 + if k == 0 { 127 } else { k as u32 };
            msg.set_label_font(FONT);
            msg.set_label(&bar(0, total));
            let (mut m, b) = (msg.clone(), busy.clone());
            app::add_timeout3(0.1, move |h| {
                if b.get() {
                    m.set_label(&bar(sign::done(), total));
                    app::repeat_timeout3(0.1, h);
                }
            });
            let (mut buf, mut tbuf, mut msg, busy, set_dot) = (
                buf.clone(),
                tbuf.clone(),
                msg.clone(),
                busy.clone(),
                set_dot.clone(),
            );
            let saved = name.clone();
            bg(
                move || sign::read(&name, n, k).map_err(|e| e.to_string()),
                move |r| {
                    busy.set(false);
                    msg.set_label_font(Font::Helvetica);
                    set_dot(r.is_ok());
                    if r.is_ok() {
                        cfg_set("port", &saved);
                    }
                    match r {
                        Ok(pages) => {
                            tbuf.set_text(&decode(&pages[0]));
                            buf.set_text(&decode(&pages[1..].concat()));
                            msg.set_label("Odczytano");
                        }
                        Err(e) => msg.set_label(&format!("Błąd odczytu: {e}")),
                    }
                },
            );
        }
    });
    put_sign.set_callback({
        let (port, ports, num, pgs, mut msg, buf, tbuf, busy, set_dot) = (
            port.clone(),
            ports.clone(),
            num.clone(),
            pgs.clone(),
            msg.clone(),
            buf.clone(),
            title_buf.clone(),
            busy.clone(),
            set_dot.clone(),
        );
        move |_| {
            if busy.get() {
                return;
            }
            let Some(name) = port_of(&port, &ports) else {
                set_dot(false);
                msg.set_label("Wybierz port szeregowy");
                return;
            };
            let (pages, rep) = match to_pages(&tbuf.text(), &buf.text()) {
                Ok(r) => r,
                Err(e) => return msg.set_label(&e),
            };
            let n = num.value() as u16;
            busy.set(true);
            msg.set_label("Zapis do tablicy...");
            let (mut pgs, mut msg, busy, set_dot) =
                (pgs.clone(), msg.clone(), busy.clone(), set_dot.clone());
            let count = pages.len() - 1;
            let saved = name.clone();
            bg(
                move || sign::write(&name, n, &pages).map_err(|e| e.to_string()),
                move |r| {
                    busy.set(false);
                    set_dot(r.is_ok());
                    if r.is_ok() {
                        cfg_set("port", &saved);
                    }
                    match r {
                        Ok(()) => {
                            pgs.set_value(count as u32);
                            msg.set_label(&if rep > 0 {
                                format!("Zapisano w tablicy; zamieniono znaki: {rep}")
                            } else {
                                "Zapisano w tablicy".to_string()
                            });
                        }
                        Err(e) => msg.set_label(&format!("Błąd zapisu: {e}")),
                    }
                },
            );
        }
    });

    app.run().unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cfg_parse_keeps_other_keys() {
        let t = parse_set("theme=dark\r\nport = COM3\r\n", "port", "COM4");
        assert_eq!(parse_get(&t, "port").as_deref(), Some("COM4"));
        assert_eq!(parse_get(&t, "theme").as_deref(), Some("dark"));
        assert_eq!(parse_get(&t, "seasonal"), None);
        assert_eq!(parse_get("COM3", "port"), None); // legacy bare-port file: ignored, not a crash
    }

    #[test]
    fn file_roundtrip_keeps_page_shape() {
        let body = (1..=10)
            .map(|i| format!("linia {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let (pages, rep) = to_pages("TYTUŁ", &body).unwrap();
        assert_eq!(rep, 0);
        assert_eq!(pages.len(), 3); // meta + 2 body pages
        let bytes = file_bytes(&pages);
        let lines: Vec<&[u8]> = bytes.split(|&b| b == b'\n').collect();
        assert_eq!(lines.len(), 3 * 11 + 1); // 3 header lines + 8 rows per page, + trailing empty
        assert!(lines.iter().all(|l| l.len() <= COLS + 1)); // + '\r'
        let back = load(&bytes).unwrap();
        assert_eq!(back.title, "TYTUŁ");
        assert_eq!(back.body, body);
        assert_eq!((back.soft, back.hard), (0, 0));
    }

    #[test]
    fn loads_without_decoration() {
        let l = load(b"abc\r\ndef\r\n").unwrap();
        assert_eq!(l.body, "abc\ndef");
        assert_eq!(l.title, default_title());
        // header lines without DECO, and a stray DECO line
        let l = load(b"T\n<- Strona nr: 001 ->\nx\n<------------------>\n<- Strona nr: 002 ->\ny")
            .unwrap();
        assert_eq!(l.title, "T");
        assert_eq!(l.body, format!("x{}y", "\n".repeat(8)));
    }

    #[test]
    fn long_lines_are_wrapped() {
        let l = load(b"aaaa bbbb cccc dddd eeee").unwrap();
        assert_eq!(
            (l.body.as_str(), l.soft, l.hard),
            ("aaaa bbbb cccc dddd\neeee", 1, 0)
        );
        let l = load(b"123456789012345678901").unwrap();
        assert_eq!(
            (l.body.as_str(), l.soft, l.hard),
            ("12345678901234567890\n1", 0, 1)
        );
        let l = load("zażółć gęślą jaźń zażółć".as_bytes()).unwrap(); // UTF-8 input
        assert!(l.body.lines().all(|x| x.chars().count() <= COLS));
        let l =
            load(b"\xbf\xf3\xb3w \xbf\xf3\xb3w \xbf\xf3\xb3w \xbf\xf3\xb3w \xbf\xf3\xb3w").unwrap(); // cp1250
        assert!(l.body.contains("żółw"));
    }
}
