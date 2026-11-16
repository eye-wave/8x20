#![cfg_attr(windows, windows_subsystem = "windows")]
use encoding_rs::WINDOWS_1250;
use fltk::{dialog::*, enums::*, frame::Frame, misc::Spinner, prelude::*, *};
use std::{cell::Cell, cell::RefCell, fs, path::PathBuf, rc::Rc, sync::mpsc};

mod sign;

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

fn lbl(text: &str, x: i32, y: i32, w: i32, h: i32, size: i32) -> Frame {
    let mut f = Frame::new(x, y, w, h, None);
    f.set_label(text);
    f.set_label_size(size);
    f.set_align(Align::Left | Align::Inside);
    f
}

fn spin(x: i32, y: i32, val: f64) -> Spinner {
    let mut s = Spinner::new(x, y, 52, 24, None);
    s.set_range(0.0, 999.0);
    s.set_value(val);
    s
}

const COLS: usize = 20; // max characters per line
const ROWS: i32 = 8; // lines per page
const FONT: Font = Font::Courier; // monospace
const SIZE: i32 = 30;

// Editor box is exactly COLS x ROWS character cells (fltk text margins: 3px sides, 1px top/bottom)
// plus a vertical scrollbar of `bar` px (1 = effectively none).
fn editor(x: i32, y: i32, bar: i32, show: Rc<Cell<bool>>) -> (text::TextEditor, text::TextBuffer) {
    draw::set_font(FONT, SIZE);
    let w = COLS as i32 * draw::width("M").round() as i32 + 6 + bar;
    let h = ROWS * draw::height() + 2;
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
    // Invisible characters: fltk can't show them, so paint · / ¶ over the visible range after the base draw.
    ed.super_draw_first(true);
    ed.draw(move |e| {
        let Some(b) = e.buffer().filter(|_| show.get()) else {
            return;
        };
        let (from, to) = (
            e.xy_to_position(e.x(), e.y(), text::PositionType::Cursor),
            e.xy_to_position(e.x() + e.w(), e.y() + e.h(), text::PositionType::Cursor),
        );
        draw::push_clip(e.x(), e.y(), e.w(), e.h());
        draw::set_font(FONT, SIZE);
        draw::set_draw_color(Color::from_rgb(160, 160, 160));
        let (cw, lh) = (draw::width("M").round() as i32, draw::height());
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
        draw::pop_clip();
    });
    (ed, buf)
}

#[derive(Clone, Copy)]
enum Al {
    Left,
    Center,
    Right,
    Justify,
}

// Plain text has no alignment attribute, so alignment is real padding: a line's leading/trailing
// spaces are layout, never content, and are recomputed here. Every mode first normalizes inner space
// runs to one space (otherwise a justified line could never be left/centered/right-aligned again).
fn align_line(l: &str, a: Al) -> String {
    let w: Vec<&str> = l.split(' ').filter(|s| !s.is_empty()).collect();
    let t = w.join(" ");
    let room = COLS.saturating_sub(t.chars().count());
    match a {
        Al::Left => t,
        Al::Center => format!("{}{t}", " ".repeat(room / 2)),
        Al::Right => format!("{}{t}", " ".repeat(room)),
        Al::Justify => {
            if w.len() < 2 {
                return t;
            }
            let gaps = w.len() - 1;
            let sp = COLS.saturating_sub(w.iter().map(|s| s.chars().count()).sum());
            let mut out = String::new();
            for (i, s) in w.iter().enumerate() {
                out += s;
                if i < gaps {
                    out += &" ".repeat(sp / gaps + (i < sp % gaps) as usize);
                }
            }
            out
        }
    }
}

// Align the selected lines (or the caret's line).
fn align(e: &mut text::TextEditor, b: &mut text::TextBuffer, a: Al) {
    let p = e.insert_position();
    let (s, z) = b
        .selection_position()
        .filter(|(s, z)| s != z)
        .unwrap_or((p, p));
    let (s, z) = (s.min(z), s.max(z));
    let z = if z > s && b.line_start(z) == z {
        z - 1
    } else {
        z
    }; // selection ending at a line start
    let (ls, le) = (b.line_start(s), b.line_end(z));
    let new = b
        .text_range(ls, le)
        .unwrap_or_default()
        .split('\n')
        .map(|l| align_line(l, a))
        .collect::<Vec<_>>()
        .join("\n");
    b.replace(ls, le, &new);
    b.unselect();
    e.set_insert_position(ls + new.len() as i32);
    e.show_insert_position();
}

// Insert text at the caret, enforcing COLS per line and (optionally) a max line count.
// Typing is blocked at the limit; pasting hard-wraps at the limit instead.
fn put(
    e: &mut text::TextEditor,
    b: &mut text::TextBuffer,
    s: &str,
    max_lines: Option<i32>,
    wrap: bool,
) {
    if let Some((a, z)) = b.selection_position() {
        if a != z {
            b.remove(a, z);
            e.set_insert_position(a);
        }
        b.unselect();
    }
    let mut pos = e.insert_position();
    for ch in s.chars() {
        let ch = if ch == '\t' { ' ' } else { ch };
        if ch == '\r' || (ch != '\n' && ch.is_control()) {
            continue;
        }
        let (ls, le) = (b.line_start(pos), b.line_end(pos));
        let len = b.text_range(ls, le).unwrap_or_default().chars().count();
        let room = max_lines.is_none_or(|m| b.count_lines(0, b.length()) + 1 < m);
        let out = if ch == '\n' {
            if !room {
                continue;
            }
            "\n".to_string()
        } else if len < COLS {
            ch.to_string()
        } else if wrap && pos == le && room && ch != ' ' {
            format!("\n{ch}")
        } else {
            continue;
        };
        b.insert(pos, &out);
        pos += out.len() as i32;
    }
    e.set_insert_position(pos);
    e.show_insert_position();
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

// Inverse of file_bytes. Rows before the first header belong to page 0; a bad row or page is an error, never truncated.
fn parse(txt: &[u8]) -> Result<sign::Pages, String> {
    let mut l: Vec<&[u8]> = txt
        .split(|&b| b == b'\n')
        .map(|x| x.strip_suffix(b"\r").unwrap_or(x))
        .collect();
    if l.last().is_some_and(|x| x.is_empty()) {
        l.pop();
    }
    let (mut pages, mut cur, mut i) = (vec![vec![]], 0usize, 0);
    while i < l.len() {
        if l[i] == DECO
            && l.get(i + 2) == Some(&DECO)
            && l.get(i + 1)
                .is_some_and(|s| s.starts_with(b"<- Strona nr: "))
        {
            cur = l[i + 1]
                .get(14..17)
                .and_then(|d| std::str::from_utf8(d).ok())
                .and_then(|d| d.parse().ok())
                .ok_or(format!("Zły nagłówek strony, linia {}", i + 2))?;
            if cur > 127 {
                return Err(format!("Strona {cur} za wysoka (max 127)"));
            }
            pages.resize(pages.len().max(cur + 1), vec![]);
            i += 3;
            continue;
        }
        if l[i].len() > COLS {
            return Err(format!("Linia {} ma więcej niż {COLS} znaków", i + 1));
        }
        pages[cur].push(l[i].to_vec());
        if pages[cur].len() > ROWS as usize {
            return Err(format!(
                "Strona {cur} ma więcej niż {ROWS} linii (linia {})",
                i + 1
            ));
        }
        i += 1;
    }
    Ok(pages)
}

// Pages 1.. as one editor body; each page padded to ROWS rows so editor lines stay aligned with pages.
fn body_text(pages: &[Vec<Vec<u8>>]) -> String {
    let padded: Vec<Vec<u8>> = pages
        .iter()
        .flat_map(|p| (0..ROWS as usize).map(move |i| p.get(i).cloned().unwrap_or_default()))
        .collect();
    decode(&padded)
}

// True if the text just inserted at pos..pos+ins left any line over COLS or too many lines.
fn bad(b: &text::TextBuffer, pos: i32, ins: i32, max_lines: Option<i32>) -> bool {
    let (mut p, end) = (b.line_start(pos), b.line_end((pos + ins).min(b.length())));
    loop {
        let le = b.line_end(p);
        if b.text_range(p, le).unwrap_or_default().chars().count() > COLS {
            return true;
        }
        if le >= end || le >= b.length() {
            break;
        }
        p = le + 1;
    }
    max_lines.is_some_and(|m| b.count_lines(0, b.length()) + 1 > m)
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
        move |pos, ins, _del, _r, _t| {
            if ins > 0 && bad(&b, pos, ins, max_lines) {
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

fn port_of(c: &menu::Choice) -> Option<String> {
    if c.value() > 0 {
        c.text(c.value())
    } else {
        None
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

// Editor text -> pages: title page first, then the body in chunks of ROWS lines.
fn to_pages(title: &str, body: &str) -> Result<sign::Pages, String> {
    let rows = |t: &str| -> Result<Vec<Vec<u8>>, String> {
        t.lines()
            .map(|l| {
                let (b, _, bad) = WINDOWS_1250.encode(l);
                if bad || b.len() > COLS {
                    Err(format!(
                        "Nieprawidłowa linia (znak spoza Windows-1250?): {l}"
                    ))
                } else {
                    Ok(b.into_owned())
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
    Ok(pages)
}

fn main() {
    let app = app::App::default();
    app::set_visible_focus(false);
    app::set_font_size(14);

    let mut win = window::Window::default()
        .with_size(960, 728)
        .with_label("Notatnik");
    win.set_border(false);
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

    let mut min = btn!("min", 836, 12);
    let mut max = btn!("max", 882, 12);
    let mut close = btn!("close", 928, 12);
    min.set_callback({
        let mut w = win.clone();
        move |_| w.iconize()
    });
    max.set_callback({
        let mut w = win.clone();
        move |_| {
            if w.maximize_active() {
                w.un_maximize()
            } else {
                w.maximize()
            }
        }
    });
    close.set_callback(|_| app::quit());

    // toolbar
    let mut new = btn!("nowy", 19, 82);
    let mut open = btn!("otworz", 87, 82);
    let mut save = btn!("zapisz", 155, 82);
    let mut get = btn!("odczytaj", 223, 82);
    let mut put_sign = btn!("wyslij", 291, 82);
    let num = spin(725, 7, 1.0);
    let pgs = spin(725, 35, 0.0);
    btn!("ustaw", 746, 82);
    btn!("opcje", 814, 82);
    let mut info = btn!("info", 882, 82);
    let (mut left, mut center, mut right, mut just) = (
        btn!("lewo", 363, 82),
        btn!("srodek", 431, 82),
        btn!("prawo", 499, 82),
        btn!("justuj", 567, 82),
    );
    let mut marks = btn!("znaki", 635, 82);
    info.set_callback(|_| message_default("Notatnik – prosty edytor tekstu"));

    let show = Rc::new(Cell::new(false));

    // left panel: title page
    let kw1 = lbl("K: 1   W: 1", 330, 183, 120, 34, 14);
    let (mut title_ed, title_buf) = editor(36, 227, 1, show.clone());
    rules(&mut title_ed, &title_buf, Some(ROWS), kw1, None);

    // right panel: page
    let page = lbl("Strona - 001", 522, 183, 200, 34, 17);
    let kw2 = lbl("K: 1   W: 1", 740, 183, 120, 34, 14);
    let mut search = btn!("szukaj", 898, 186);
    let (mut ed, buf) = editor(486, 227, 12, show.clone());
    rules(&mut ed, &buf, None, kw2, Some(page));

    // status bar
    let com = lbl("COM-", 70, 676, 80, 24, 16);
    let msg = lbl("", 170, 676, 540, 24, 13);
    let mut port = menu::Choice::new(730, 674, 210, 26, None);
    port.add_choice("Port szeregowy niewybrany");
    for name in sign::ports() {
        port.add_choice(&name);
    }
    port.set_value(0);
    port.set_text_size(13);
    port.set_callback({
        let mut com = com.clone();
        move |p| com.set_label(&port_of(p).unwrap_or_else(|| "COM-".into()))
    });

    root.end();
    win.end();
    win.resize_callback(move |_, _, _, w, h| root.set_pos((w - 960) / 2, (h - 728) / 2));
    win.show();

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
            b.set_text("");
            tb.set_text("");
            *path.borrow_mut() = None;
        }
    });
    open.set_callback({
        let (mut b, mut tb, path) = (buf.clone(), title_buf.clone(), path.clone());
        move |_| {
            let mut d = NativeFileChooser::new(NativeFileChooserType::BrowseFile);
            d.show();
            let f = d.filename();
            if f.as_os_str().is_empty() {
                return;
            }
            match fs::read(&f)
                .map_err(|e| e.to_string())
                .and_then(|t| parse(&t))
            {
                Ok(pages) => {
                    tb.set_text(&decode(&pages[0]));
                    b.set_text(&body_text(&pages[1..]));
                    *path.borrow_mut() = Some(f);
                }
                Err(e) => alert_default(&format!("Nie można otworzyć: {e}")),
            }
        }
    });
    save.set_callback({
        let (b, tb, path) = (buf.clone(), title_buf.clone(), path.clone());
        move |_| {
            let bytes = match to_pages(&tb.text(), &b.text()) {
                Ok(p) => file_bytes(&p),
                Err(e) => return alert_default(&format!("Nie można zapisać: {e}")),
            };
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
            if let Err(e) = fs::write(&p, bytes) {
                alert_default(&format!("Nie można zapisać: {e}"));
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
    get.set_callback({
        let (port, num, pgs, mut msg, buf, tbuf, busy) = (
            port.clone(),
            num.clone(),
            pgs.clone(),
            msg.clone(),
            buf.clone(),
            title_buf.clone(),
            busy.clone(),
        );
        move |_| {
            if busy.get() {
                return;
            }
            let Some(name) = port_of(&port) else {
                msg.set_label("Wybierz port szeregowy");
                return;
            };
            let (n, k) = (num.value() as u16, pgs.value() as u8);
            busy.set(true);
            msg.set_label("Odczyt z tablicy...");
            let (mut buf, mut tbuf, mut msg, busy) =
                (buf.clone(), tbuf.clone(), msg.clone(), busy.clone());
            bg(
                move || sign::read(&name, n, k).map_err(|e| e.to_string()),
                move |r| {
                    busy.set(false);
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
        let (port, num, pgs, mut msg, buf, tbuf, busy) = (
            port.clone(),
            num.clone(),
            pgs.clone(),
            msg.clone(),
            buf.clone(),
            title_buf.clone(),
            busy.clone(),
        );
        move |_| {
            if busy.get() {
                return;
            }
            let Some(name) = port_of(&port) else {
                msg.set_label("Wybierz port szeregowy");
                return;
            };
            let pages = match to_pages(&tbuf.text(), &buf.text()) {
                Ok(p) => p,
                Err(e) => return msg.set_label(&e),
            };
            let n = num.value() as u16;
            busy.set(true);
            msg.set_label("Zapis do tablicy...");
            let (mut pgs, mut msg, busy) = (pgs.clone(), msg.clone(), busy.clone());
            let count = pages.len() - 1;
            bg(
                move || sign::write(&name, n, &pages).map_err(|e| e.to_string()),
                move |r| {
                    busy.set(false);
                    match r {
                        Ok(()) => {
                            pgs.set_value(count as f64);
                            msg.set_label("Zapisano w tablicy");
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
    fn alignment() {
        let sp = |n| " ".repeat(n);
        assert_eq!(align_line("abcd", Al::Center), format!("{}abcd", sp(8)));
        assert_eq!(align_line("  abcd  ", Al::Right), format!("{}abcd", sp(16)));
        assert_eq!(align_line("  abcd  ", Al::Left), "abcd");
        assert_eq!(align_line("ab cd", Al::Justify), format!("ab{}cd", sp(16)));
        assert_eq!(
            align_line(" a b c ", Al::Justify),
            format!("a{}b{}c", sp(9), sp(8))
        );
        assert_eq!(align_line("word", Al::Justify), "word");
        let j = align_line("ab cd", Al::Justify); // justified lines must be undoable
        assert_eq!(align_line(&j, Al::Left), "ab cd");
        assert_eq!(align_line(&j, Al::Center), format!("{}ab cd", sp(7)));
    }

    #[test]
    fn file_roundtrip_keeps_page_shape() {
        let body = (1..=10)
            .map(|i| format!("linia {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let pages = to_pages("TYTUŁ", &body).unwrap();
        assert_eq!(pages.len(), 3); // meta + 2 body pages
        let bytes = file_bytes(&pages);
        let lines: Vec<&[u8]> = bytes.split(|&b| b == b'\n').collect();
        assert_eq!(lines.len(), 3 * 11 + 1); // 3 header lines + 8 rows per page, + trailing empty
        assert!(lines.iter().all(|l| l.len() <= COLS + 1)); // + '\r'
        let back = parse(&bytes).unwrap();
        assert_eq!(decode(&back[0]), "TYTUŁ");
        assert_eq!(body_text(&back[1..]), body);
        assert!(parse(b"123456789012345678901").is_err()); // 21 chars
    }
}
