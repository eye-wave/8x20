#![cfg_attr(windows, windows_subsystem = "windows")]
use encoding_rs::WINDOWS_1250;
use fltk::{dialog::*, enums::*, frame::Frame, prelude::*, *};
use std::{cell::Cell, cell::RefCell, fs, path::PathBuf, rc::Rc, sync::mpsc};

mod engine;
mod numinput;
mod sign;

use numinput::NumInput;

use engine::{Al, COLS, ROWS, align, align_line, bad, kill, put, rejoin, tidy, valign};

fn png(name: &str, d: &[u8]) -> image::PngImage {
    image::PngImage::from_data(d).unwrap_or_else(|e| panic!("tex/{name}.png: {e}"))
}

// Image button from tex/<name>_off.png + tex/<name>_on.png (on = hovered); the PNG sets the size.
fn btn(off: image::PngImage, on: image::PngImage, x: i32, y: i32) -> button::Button {
    let mut b = button::Button::new(x, y, off.w(), off.h(), None);
    b.set_frame(FrameType::NoBox);
    b.set_image(Some(off.clone()));
    b.handle(move |b, ev| match ev {
        Event::Enter | Event::Leave => {
            b.set_image(Some(if ev == Event::Enter {
                on.clone()
            } else {
                off.clone()
            }));
            b.redraw();
            true
        }
        _ => false,
    });
    b
}

macro_rules! btn {
    ($n:literal, $x:expr, $y:expr) => {
        btn(
            png(
                concat!($n, "_off"),
                include_bytes!(concat!("../tex/", $n, "_off.png")),
            ),
            png(
                concat!($n, "_on"),
                include_bytes!(concat!("../tex/", $n, "_on.png")),
            ),
            $x,
            $y,
        )
    };
}

// Decorative separator (tex/sep.png), vertically centred on cy.
fn sep(img: &image::PngImage, x: i32, cy: i32) {
    let mut f = Frame::new(x, cy - img.h() / 2, img.w(), img.h(), None);
    f.set_image(Some(img.clone()));
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
        draw::set_draw_color(Color::from_rgb(120, 120, 120));
        for r in 0..=e.h() / lh + 1 {
            if pages && (n0 + r) % ROWS == 0 && n0 + r > 0 {
                draw::draw_line(e.x(), y0 + r * lh, e.x() + e.w() - 12, y0 + r * lh);
            }
        }
        if show.get() {
            let to = e.xy_to_position(e.x() + e.w(), e.y() + e.h(), text::PositionType::Cursor);
            draw::set_draw_color(Color::from_rgb(160, 160, 160));
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

// Last port that worked, remembered between launches (best effort, errors ignored).
fn cache() -> PathBuf {
    let dir = std::env::var_os("APPDATA").or_else(|| std::env::var_os("HOME"));
    PathBuf::from(dir.unwrap_or_else(|| ".".into())).join(".tablica_port")
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
        if matches!(ev, Event::KeyUp | Event::Released | Event::Focus) {
            upd(e);
        }
        false
    });
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

    let mut win = window::Window::default()
        .with_size(960, 728)
        .with_label("Notatnik");
    win.set_border(false);
    win.set_icon(Some(png("icon", include_bytes!("../tex/icon.png"))));
    win.size_range(960, 728, 0, 0); // resizable by the window manager (needed for maximize)

    // All content lives in one fixed-size group that is centered when the window grows.
    let mut root = group::Group::new(0, 0, 960, 728, None);

    // backdrop: title bar, toolbar/page panels, captions and editor frames are baked into one texture
    let mut backdrop = Frame::new(0, 0, 960, 728, None);
    backdrop.set_image(Some(png("bg", include_bytes!("../tex/bg.png"))));

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

    // window buttons (the PNGs are baked onto bg.png at exactly these positions)
    let mut min = btn!("min", 882, 12);
    let mut close = btn!("close", 928, 12);
    min.set_callback({
        let mut w = win.clone();
        move |_| w.iconize()
    });
    close.set_callback(|_| app::quit());

    // toolbar
    let mut new = btn!("nowy", 19, 82);
    let mut open = btn!("otworz", 87, 82);
    let mut save = btn!("zapisz", 155, 82);
    let mut get = btn!("odczytaj", 223, 82);
    let mut put_sign = btn!("wyslij", 291, 82);
    // interrupt button: the wyslij texture again, right after it (must be made before root.end())
    let mut stop = btn!("przerwij", 359, 82);
    // slot number above read, page count above write (centred over the 64px buttons)
    let mut num = NumInput::new(229, 56, 0, 999, false);
    num.set_value(1);
    let pgs = NumInput::new(297, 56, 0, 127, true); // 0 = auto
    btn!("ustaw", 814, 82);
    let mut info = btn!("info", 882, 82);
    // Groups left to right: files (64px) | horizontal align (36px each, 4px apart) |
    // vertical align (top / centred) | invisibles | settings. The groups after the files use
    // 12px gutters around a 2px separator because the toolbar is tight.
    let (mut left, mut center, mut right, mut just) = (
        btn!("lewo", 449, 100),
        btn!("srodek", 489, 100),
        btn!("prawo", 529, 100),
        btn!("justuj", 569, 100),
    );
    let (mut vtop, mut vmid) = (btn!("pion_gora", 631, 100), btn!("pion_srodek", 671, 100));
    let mut marks = btn!("znaki", 733, 100);
    let sp = png("sep", include_bytes!("../tex/sep.png"));
    for x in [435, 617, 719, 793] {
        sep(&sp, x, 118);
    }
    info.set_callback(|_| message_default("Notatnik – prosty edytor tekstu"));

    let show = Rc::new(Cell::new(false));

    // left panel: title page
    let kw1 = lbl("K: 1   W: 1", 290, 183, 120, 34, 14);
    let (mut title_ed, mut title_buf) = editor(36, 227, 1, 8, show.clone(), false);
    rules(&mut title_ed, &title_buf, Some(ROWS), kw1, None);
    title_buf.set_text(&default_title());

    // right panel: page
    let page = lbl("Strona - 001", 522, 183, 200, 34, 17);
    let kw2 = lbl("K: 1   W: 1", 740, 183, 120, 34, 14);
    let mut search = btn!("szukaj", 898, 186);
    let (mut ed, buf) = editor(486, 227, 12, 48, show.clone(), true);
    rules(&mut ed, &buf, None, kw2, Some(page));

    // status bar
    let mut com = lbl("COM-", 60, 676, 100, 24, 16);
    com.set_label_color(Color::from_hex(0xEED590));
    let mut msg = lbl("", 330, 676, 370, 24, 13); // right of the decoration, ends before the status dot
    msg.set_label_color(Color::from_hex(0xEED590)); // same as the COM label
    // connection status dot, left of the port dropdown: green after a successful read/write,
    // red after a failure and whenever another port is picked
    let (dot_on, dot_off) = (
        png("status_dot_on", include_bytes!("../tex/status_dot_on.png")),
        png(
            "status_dot_off",
            include_bytes!("../tex/status_dot_off.png"),
        ),
    );
    let mut dot = Frame::new(709, 679, 15, 15, None);
    dot.set_image(Some(dot_off.clone()));
    let set_dot: Rc<dyn Fn(bool)> = Rc::new(move |ok| {
        let mut d = dot.clone();
        d.set_image(Some(if ok { dot_on.clone() } else { dot_off.clone() }));
        d.redraw();
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
    if let Some(i) = fs::read_to_string(cache())
        .ok()
        .and_then(|s| ports.iter().position(|p| *p == s.trim()))
    {
        port.set_value(i as i32 + 1);
        com.set_label(&ports[i]);
    }

    root.end();
    win.end();
    win.resize_callback(move |_, _, _, w, h| root.set_pos((w - 960) / 2, (h - 728) / 2));
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
    open.set_callback({
        let (mut b, mut tb, path, mut msg) =
            (buf.clone(), title_buf.clone(), path.clone(), msg.clone());
        move |_| {
            let mut d = NativeFileChooser::new(NativeFileChooserType::BrowseFile);
            d.show();
            let f = d.filename();
            if f.as_os_str().is_empty() {
                return;
            }
            match fs::read(&f)
                .map_err(|e| e.to_string())
                .and_then(|t| load(&t))
            {
                Ok(l) => {
                    tb.set_text(&l.title);
                    b.set_text(&l.body);
                    *path.borrow_mut() = Some(f);
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
    });
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
            msg.set_label("Odczyt z tablicy...");
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
                    set_dot(r.is_ok());
                    if r.is_ok() {
                        let _ = fs::write(cache(), saved);
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
                        let _ = fs::write(cache(), saved);
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
