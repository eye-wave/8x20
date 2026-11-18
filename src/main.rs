#![cfg_attr(windows, windows_subsystem = "windows")]
use encoding_rs::WINDOWS_1250;
use fltk::{dialog::*, enums::*, frame::Frame, prelude::*, *};
use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, LazyLock, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc,
    },
};

mod batch;
mod cfg;
mod dnd;
mod editor;
mod engine;
mod file;
mod numinput;
mod overlay;
mod psalms;
mod sign;
mod theme;
mod util;
mod widgets;

use batch::*;
use cfg::*;
use dnd::*;
use editor::*;
use engine::{Al, COLS, ROWS, align, align_line, bad, kill, put, rejoin, tidy, valign};
use file::*;
use numinput::NumInput;
use overlay::*;
use psalms::*;
use theme::{
    CUR, THEMES, Theme, apply_theme, img, pick_theme, png, seasonal_on, skin, theme, tint,
};
use util::*;
use widgets::*;

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
    editor::load_mono(); // before any widget measures FONT

    CUR.store(pick_theme(), Relaxed); // seasonal / saved theme, before any widget is built
    tint();
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

    info.set_callback(|_| about(INFO));

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
    let ok = Arc::new(AtomicBool::new(false));
    let mut dot = Frame::new(709, 679, 15, 15, None);
    paint_dot(&mut dot, theme(), false);
    skin({
        let (mut d, ok) = (dot.clone(), ok.clone());
        move |t| paint_dot(&mut d, t, ok.load(Relaxed))
    });
    let set_dot: Rc<dyn Fn(bool)> = Rc::new(move |v| {
        ok.store(v, Relaxed);
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
    // the closed box and the popup list both use these (text on the editors' background)
    let paint = |p: &mut menu::Choice, t: &Theme| {
        p.set_color(Color::from_hex(t.colors.text_bg));
        p.set_text_color(Color::from_hex(t.colors.text));
        p.set_selection_color(Color::from_hex(t.colors.accent));
        p.redraw();
    };
    // FLTK blends the closed box's face with its own grays whatever colour is set, so draw it ourselves
    port.super_draw_first(false);
    port.draw(|p| {
        let c = &theme().colors;
        let (x, y, w, h) = (p.x(), p.y(), p.w(), p.h());
        draw::draw_rect_fill(x, y, w, h, Color::from_hex(c.text_bg));
        draw::draw_rect_with_color(x, y, w, h, Color::from_hex(c.page_line));
        draw::set_draw_color(Color::from_hex(c.text));
        draw::set_font(p.text_font(), p.text_size());
        draw::draw_text2(
            &p.choice().unwrap_or_default(),
            x + 6,
            y,
            w - 30,
            h,
            Align::Left,
        );
        let (ax, ay) = (x + w - 16, y + h / 2 - 2);
        draw::draw_polygon(ax, ay, ax + 8, ay, ax + 4, ay + 5);
    });
    paint(&mut port, theme());
    skin({
        let mut p = port.clone();
        move |t| paint(&mut p, t)
    });
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
    let psalms = psalms_overlay(&ed);
    let busy = Rc::new(Cell::new(false)); // one serial job at a time, shared with the batch dialog
    let batch = batch_overlay(&ed, &port, &ports, &set_dot, &busy);
    let overlay = more_overlay(&ed, &psalms, &batch);
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
    // Windows (fltk) sends the drop's FL_PASTE straight to Fl::belowmouse(), which stays unset while this
    // handler runs before the children; so claim it, or the drop is silently lost.
    win.handle(|w, ev| {
        if matches!(ev, Event::DndEnter | Event::DndDrag) {
            app::set_belowmouse(w);
        }
        dnd(ev)
    });
    win.show();
    #[cfg(windows)]
    crate::util::taskbar(&win);
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
    new.set_callback({
        let (mut b, mut tb) = (buf.clone(), title_buf.clone());
        move |_| {
            // a project that is empty (or still the untouched default) is replaced without asking
            let dirty = !b.text().is_empty() || tb.text() != default_title();
            if dirty
                && ask(
                    "Bieżący tekst zostanie usunięty. Utworzyć nowy?",
                    &["Nie", "Tak"],
                ) != Some(1)
            {
                return;
            }
            b.set_text("");
            tb.set_text(&default_title());
            *CUR_FILE.lock().unwrap() = None;
        }
    });
    let open_path = Rc::new({
        let (b, tb, msg) = (buf.clone(), title_buf.clone(), msg.clone());
        move |f: PathBuf| open_file(&f, &mut b.clone(), &mut tb.clone(), &mut msg.clone())
    });
    let (drop_tx, drop_rx) = mpsc::channel();
    let _ = DROPS.set(drop_tx);
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
        let (b, tb, mut msg) = (buf.clone(), title_buf.clone(), msg.clone());
        move |_| {
            let (pages, rep) = match to_pages(&tb.text(), &b.text()) {
                Ok(r) => r,
                Err(e) => return popup(&format!("Nie można zapisać: {e}")),
            };
            let bytes = file_bytes(&pages);
            let p = {
                let mut cur = CUR_FILE.lock().unwrap();
                if cur.is_none() {
                    let mut d = NativeFileChooser::new(NativeFileChooserType::BrowseSaveFile);
                    d.set_option(NativeFileChooserOptions::SaveAsConfirm);
                    d.show();
                    let f = d.filename();
                    if f.as_os_str().is_empty() {
                        return;
                    }
                    *cur = Some(f);
                }
                cur.clone().unwrap()
            };
            match fs::write(&p, bytes) {
                Ok(()) => msg.set_label(&if rep > 0 {
                    format!("Zapisano (Windows-1250); zamieniono znaki: {rep}")
                } else {
                    "Zapisano".to_string()
                }),
                Err(e) => popup(&format!("Nie można zapisać: {e}")),
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
            if ask(&format!("Nadpisać slot {n:03} w tablicy?"), &["Nie", "Tak"]) != Some(1) {
                return;
            }
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

    // `run`, plus opening what was dropped on the window
    while app.wait() {
        while let Ok(f) = drop_rx.try_recv() {
            open_path(f);
        }
    }
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
