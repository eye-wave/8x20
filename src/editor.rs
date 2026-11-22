use super::*;

pub const FONT: Font = Font::Courier; // monospace: slot is remapped to the embedded font by `load_mono`
pub const SIZE: i32 = 30;

// Editor box is exactly COLS x ROWS character cells (fltk text margins: 3px sides, 1px top/bottom)
// plus a vertical scrollbar of `bar` px (1 = effectively none).
pub fn editor(
    x: i32,
    y: i32,
    bar: i32,
    slack: i32,
    show: Rc<Cell<bool>>,
    pages: bool, // draw page separators
) -> (text::TextEditor, text::TextBuffer) {
    draw::set_font(FONT, SIZE);
    // +2px text slack so a full 20-char line (and the caret after it) never triggers fltk's
    // horizontal scroll (Windows glyph advance is a hair over the rounded width); taken from the bar.
    let pad = if bar > 2 { 2 } else { 0 };
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
    ed.set_scrollbar_size(bar - pad);
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

// Polish right-click menu for the editors (FLTK has none for text editors).
pub fn ctx(e: &mut text::TextEditor) {
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

// Enforces the line rules on the buffer itself (so it works however fltk orders its key handling):
// any edit that breaks them is taken back on the next loop tick; pastes are re-inserted hard-wrapped.
// Also keeps K (column) / W (row) and the page label fresh.
pub fn rules(
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

// Embedded Noto Sans Mono (Latin + Latin Extended, OFL, see font/OFL.txt): the same 0.6em x 1.362em cell on every OS,
// so the editor matches the background art on Windows too (its Courier New fallback is much shorter per line).
// fltk can only load fonts from a file, so it goes through the temp dir; falls back to system Courier on failure.
pub fn load_mono() {
    let p = std::env::temp_dir().join("tablica-mono.ttf");
    if fs::write(&p, include_bytes!("../font/mono.ttf")).is_ok()
        && let Ok(name) = Font::load_font(&p)
    {
        Font::set_font(FONT, &name);
    }
}
