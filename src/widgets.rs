use super::*;

// Image button from <name>_off.svg + <name>_on.svg (on = hovered); the SVG's width/height set the size,
// so every theme's textures must have the same sizes.
pub fn btn(name: &'static str, x: i32, y: i32) -> button::Button {
    // no cached state: the images are rebuilt from the current theme whenever they are needed
    let face =
        move |t: &Theme, on: bool| img(t, &format!("{name}_{}", if on { "on" } else { "off" }));
    let i = face(theme(), false);
    let mut b = button::Button::new(x, y, i.w(), i.h(), None);
    b.set_frame(FrameType::NoBox);
    b.set_image(Some(i));
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
    let mut w = window::Window::default()
        .with_size(360.max(row + 32), 150)
        .with_label("Tablica")
        .center_screen();
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
