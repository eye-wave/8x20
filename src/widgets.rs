use super::*;

// Window button (min/close): <n>_off.svg + <n>_on.svg (on = hovered); the SVG's width/height set the size.
pub fn win_btn(name: &'static str, x: i32, y: i32) -> button::Button {
    // no cached state: the images are rebuilt from the current theme whenever they are needed
    let face =
        move |t: &Theme, on: bool| img(t, &format!("{name}_{}", if on { "on" } else { "off" }));
    let i = face(theme(), false);
    let mut b = button::Button::new(x, y, i.w(), i.h(), None);
    b.set_frame(FrameType::NoBox);
    b.set_image(Some(i));
    b.super_draw_first(false); // backdrop first, then the default draw puts the face on it
    b.draw(|b| bg_under(&*b));
    b.handle(move |b, ev| match ev {
        Event::Enter | Event::Leave => {
            b.set_image(Some(face(theme(), ev == Event::Enter)));
            b.redraw();
            true
        }
        _ => false,
    });
    skin({
        let mut b = b.clone();
        move |t| {
            b.set_image(Some(face(t, false)));
            b.redraw();
        }
    });
    b
}

// Label/icon layout of a face button; the size itself comes from the background SVG.
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Tool,     // 63x71: icon, label centred under it, everything sinks 1px up when hovered
    Icon,     // 35x36 / 27x28: icon only
    Download, // 428x44: icon and label side by side
}

// Face button = background (bg_<colour>_{off,on}.svg) + icon (icon_<name>.svg, same canvas as the
// background, drawn where it belongs) + label drawn here in code (regular, bold when hovered).
// Rebuilt from the current theme on the first draw after a theme change.
pub fn face_btn(
    bg: &'static str,
    icon: Option<&'static str>,
    label: &'static str,
    kind: Kind,
    x: i32,
    y: i32,
) -> button::Button {
    let faces = move |t: &Theme| {
        (
            [img(t, &format!("{bg}_off")), img(t, &format!("{bg}_on"))],
            icon.map(|n| [img(t, n), icon_on(t, n)]),
        )
    };
    let first = faces(theme());
    let mut b = button::Button::new(x, y, first.0[0].w(), first.0[0].h(), None);
    b.set_frame(FrameType::NoBox);
    let hot = Rc::new(Cell::new(false));
    let mut cache = (CUR.load(Relaxed), first);
    b.draw({
        let hot = hot.clone();
        move |b| {
            bg_under(&*b);
            if cache.0 != CUR.load(Relaxed) {
                cache = (CUR.load(Relaxed), faces(theme()));
            }
            let (_, (bgs, icons)) = &mut cache;
            let on = hot.get();
            let (x, y, w, h) = (b.x(), b.y(), b.w(), b.h());
            bgs[on as usize].draw(x, y, w, h);
            let sink = if on && kind == Kind::Tool { -1 } else { 0 };
            if let Some(ic) = icons {
                ic[on as usize].draw(x, y + sink, w, h);
            }
            if label.is_empty() {
                return;
            }
            // light text on the dark green "nowy" face, the theme's dark colour elsewhere
            let c = if bg == "bg_green" {
                Color::White
            } else {
                Color::from_hex(theme().colors.scrim)
            };
            draw::set_draw_color(c);
            let font = if on {
                Font::HelveticaBold
            } else {
                Font::Helvetica
            };
            let mut size = if kind == Kind::Download { 16 } else { 13 };
            draw::set_font(font, size);
            while kind == Kind::Tool && draw::width(label) > (w - 6) as f64 && size > 8 {
                size -= 1; // "Word 2003" and friends
                draw::set_font(font, size);
            }
            if kind == Kind::Tool {
                draw::draw_text2(label, x, y + 47 + sink, w, 14, Align::Center);
            } else {
                draw::draw_text2(label, x + 197, y + 10, w - 197, 20, Align::Left);
            }
        }
    });
    b.handle(move |b, ev| match ev {
        Event::Enter | Event::Leave => {
            hot.set(ev == Event::Enter);
            b.redraw();
            true
        }
        _ => false,
    });
    b
}

// 63x71 toolbar button: coloured background, icon, label.
pub fn tool_btn(
    bg: &'static str,
    icon: &'static str,
    label: &'static str,
    x: i32,
    y: i32,
) -> button::Button {
    face_btn(bg, Some(icon), label, Kind::Tool, x, y)
}

// 35x36 icon-only button (text justify, vertical align, ...).
pub fn icon_btn(icon: &'static str, x: i32, y: i32) -> button::Button {
    face_btn("bg_small", Some(icon), "", Kind::Icon, x, y)
}

// 27x28 icon-only search button.
pub fn search_btn(x: i32, y: i32) -> button::Button {
    face_btn("bg_search", Some("icon_szukaj"), "", Kind::Icon, x, y)
}

// 428x44 download button.
pub fn download_btn(x: i32, y: i32) -> button::Button {
    face_btn(
        "bg_wide",
        Some("icon_pobierz"),
        "Pobierz",
        Kind::Download,
        x,
        y,
    )
}

// Decorative separator (sep.svg), vertically centred on cy.
pub fn sep(x: i32, cy: i32) {
    let i = img(theme(), "sep");
    let mut f = Frame::new(x, cy - i.h() / 2, i.w(), i.h(), None);
    f.set_image(Some(i));
    skin(move |t| {
        f.set_image(Some(img(t, "sep")));
        f.redraw();
    });
}

pub fn paint_dot(d: &mut Frame, t: &Theme, ok: bool) {
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

pub fn default_title() -> String {
    align_line("TEKST NR 000", Al::Center)
}

pub fn lbl(text: &str, x: i32, y: i32, w: i32, h: i32, size: i32) -> Frame {
    let mut f = Frame::new(x, y, w, h, None);
    f.set_label(text);
    f.set_label_size(size);
    f.set_align(Align::Left | Align::Inside);
    f
}

// Counter labels ("K: 1   W: 1", "Strona - 001"): coloured by the theme's `page_info`.
pub fn page_info(f: Frame) -> Frame {
    let paint = |f: &mut Frame, t: &theme::Theme| {
        f.set_label_color(Color::from_hex(t.colors.page_info));
        f.redraw();
    };
    let mut c = f.clone();
    paint(&mut c, theme());
    skin(move |t| paint(&mut c, t));
    f
}

// Plain FLTK buttons in the theme's colours (accent face, dark label) instead of FLTK's gray.
pub fn paint_btn<B: ButtonExt>(b: &mut B, t: &Theme) {
    b.set_frame(FrameType::BorderBox);
    b.set_down_frame(FrameType::BorderBox);
    b.set_color(Color::from_hex(t.colors.accent));
    b.set_selection_color(Color::from_hex(t.colors.rule));
    b.set_label_color(Color::from_hex(t.colors.scrim));
    b.redraw();
}

// Same, and re-coloured on theme change (only for buttons that live as long as the app).
pub fn style_btn<B: ButtonExt + Clone + 'static + Send>(b: &mut B) {
    paint_btn(b, theme());
    let mut b = b.clone();
    skin(move |t| paint_btn(&mut b, t));
}

// Themed replacement for fltk's message/choice dialogs (gray, English "Close"); blocks until closed.
// Returns the index of the clicked button (None if the window was just closed).
pub fn ask(msg: &str, buttons: &[&str]) -> Option<usize> {
    dialog(msg, buttons, false)
}

// The "info" popup: the app icon on the left of the text.
pub fn about(msg: &str) {
    dialog(msg, &["Zamknij"], true);
}

fn dialog(msg: &str, buttons: &[&str], icon: bool) -> Option<usize> {
    let c = &theme().colors;
    let (bw, gap) = (100, 10);
    let row = buttons.len() as i32 * (bw + gap) - gap;
    let (ww, wh) = (360.max(row + 32), 150);
    // open centred over the main window, so it lands on the monitor the app is on
    let (x, y) = match app::first_window() {
        Some(m) => (m.x() + (m.w() - ww) / 2, m.y() + (m.h() - wh) / 2),
        None => (0, 0),
    };
    let mut w = window::Window::default()
        .with_pos(x, y)
        .with_size(ww, wh)
        .with_label("Tablica");
    if app::first_window().is_none() {
        w = w.center_screen();
    }
    w.set_color(Color::from_hex(c.panel));
    let tx = if icon { 96 } else { 16 };
    if icon {
        let mut i = Frame::new(16, 16, 64, 64, None);
        let mut im = png("icon", theme::ICON);
        im.scale(64, 64, true, true);
        i.set_image(Some(im));
    }
    let mut f = Frame::new(tx, 16, w.w() - tx - 16, 80, None);
    if icon {
        f.set_align(Align::Left | Align::Inside);
    }
    f.set_label(msg);
    f.set_label_color(Color::from_hex(c.scrim));
    let hit = Rc::new(Cell::new(None));
    for (i, l) in buttons.iter().enumerate() {
        let mut b = button::Button::new((w.w() - row) / 2 + i as i32 * (bw + gap), 108, bw, 28, *l);
        paint_btn(&mut b, theme());
        b.set_callback({
            let (mut w, hit) = (w.clone(), hit.clone());
            move |_| {
                hit.set(Some(i));
                w.hide()
            }
        });
    }
    w.end();
    w.make_modal(true);
    w.show();
    while w.shown() {
        app::wait();
    }
    hit.get()
}

pub fn popup(msg: &str) {
    ask(msg, &["Zamknij"]);
}
