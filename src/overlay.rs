use super::*;

pub fn close_overlay(g: &mut group::Group, ed: &mut text::TextEditor) {
    g.hide();
    MODAL.store(false, Relaxed);
    let _ = ed.take_focus();
}

// "Więcej" overlay, drawn on top of the whole UI (create it last, inside the root group). It dims what
// is behind it and swallows every mouse/key event, so nothing underneath can be touched.
// UI only for now: none of the controls do anything yet.
pub fn more_overlay(
    ed: &text::TextEditor,
    psalms_ov: &group::Group,
    batch_ov: &group::Group,
) -> group::Group {
    // Theme thumbnails sit in a grid of 3 columns; each cell is a 128x100 thumbnail + a 44px wrapped
    // label (row pitch 156). At most 2 rows are shown, further rows scroll, so the dialog never
    // outgrows the window. `dy` = how much taller the dialog is than with the old single thumbnail row.
    let rows = THEMES.len().div_ceil(3) as i32;
    let vh = rows.min(2) * 156 - 8;
    let dy = vh - 100;
    let (ox, oy) = ((960 - 460) / 2, (728 - (296 + dy)) / 2);
    let mut g = group::Group::new(0, 0, 960, 728, None);

    // dim the app behind with a 50% stipple (FLTK has no alpha fill)
    scrim();

    let mut panel = Frame::new(ox, oy, 460, 296 + dy, None);
    panel.set_frame(FrameType::BorderBox);
    panel.set_color(Color::from_hex(theme().colors.panel));

    let mut title = lbl("Dodatki", ox + 16, oy + 12, 428, 28, 20);
    title.set_label_font(Font::HelveticaBold);

    let mut psalms = button::Button::new(ox + 16, oy + 52, 208, 36, "Psalmy na miesiąc");
    style_btn(&mut psalms);
    let mut batch = button::Button::new(ox + 236, oy + 52, 208, 36, "Przetwarzanie wsadowe");
    style_btn(&mut batch);
    // either one closes this dialog and opens its own (MODAL stays on: it is still a dialog)
    for (b, next) in [(&mut psalms, psalms_ov), (&mut batch, batch_ov)] {
        b.set_callback({
            let mut next = next.clone();
            move |b| {
                if let Some(mut g) = b.parent() {
                    g.hide();
                }
                next.show();
                app::set_focus(&next);
            }
        });
    }
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
        b.set_value(i == CUR.load(Relaxed));
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
                r.set_value(i == CUR.load(Relaxed));
            }
        }
    });

    skin({
        let (mut radios, mut sc) = (radios.clone(), sc.clone());
        move |t| {
            panel.set_color(Color::from_hex(t.colors.panel));
            sc.set_color(Color::from_hex(t.colors.panel));
            sc.redraw();
            for r in radios.iter_mut() {
                r.set_selection_color(Color::from_hex(t.colors.accent));
            }
            panel.redraw();
        }
    });

    let mut close = button::Button::new(ox + 344, oy + 252 + dy, 100, 28, "Zamknij");
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
    g
}

// Dim the app behind with a 50% stipple (FLTK has no alpha fill).
pub fn scrim() {
    let mut scrim = Frame::new(0, 0, 960, 728, None);
    scrim.draw(|f| {
        draw::set_draw_color(Color::from_hex(theme().colors.scrim));
        for y in (f.y()..f.y() + f.h()).step_by(2) {
            draw::draw_line(f.x(), y, f.x() + f.w(), y);
        }
    });
}

// Esc closes; every other mouse/key event is swallowed (the group's own handler runs after its children).
pub fn modal_keys(g: &mut group::Group, ed: &text::TextEditor) {
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
}
