//! Number input without arrows: digits only, clamped to `max`; empty input snaps to `min` on blur.
//! With `auto`, 0 is shown as "auto" and a click clears it so the user can type.
use crate::theme::{Theme, skin, theme};
use fltk::{enums::*, input::Input, prelude::*};

#[derive(Clone)]
pub struct NumInput {
    i: Input,
    min: u32,
    max: u32,
    auto: bool,
}

const AUTO: &str = "Auto";

// Normalize whatever is in the box: empty / "auto" / too small -> min, too big -> max.
fn fix(i: &mut Input, min: u32, max: u32, auto: bool) {
    let n = i.value().parse::<u32>().unwrap_or(min).clamp(min, max);
    let s = if auto && n == 0 {
        AUTO.into()
    } else {
        n.to_string()
    };
    if i.value() != s {
        i.set_value(&s);
    }
}

impl NumInput {
    pub fn new(x: i32, y: i32, min: u32, max: u32, auto: bool) -> Self {
        let mut i = Input::new(x, y, 52, 24, None);
        let paint = |i: &mut Input, t: &Theme| {
            i.set_color(Color::from_hex(t.colors.text_bg));
            i.set_text_color(Color::from_hex(t.colors.text));
            i.set_cursor_color(Color::from_hex(t.colors.text));
            i.set_selection_color(Color::from_hex(t.colors.accent));
            i.redraw();
        };
        paint(&mut i, theme());
        skin({
            let mut i = i.clone();
            move |t| paint(&mut i, t)
        });
        i.set_trigger(CallbackTrigger::Changed);
        i.set_callback(move |i| {
            let d: String = i.value().chars().filter(char::is_ascii_digit).collect();
            let s = match d.parse::<u64>() {
                Ok(n) => n.min(max as u64).to_string(),
                Err(_) if d.is_empty() => String::new(),
                Err(_) => max.to_string(), // too many digits to even parse
            };
            if i.value() != s {
                i.set_value(&s);
            }
        });
        i.super_handle_first(false); // so drops reach our handler before FLTK pastes them
        i.handle(move |i, ev| {
            if crate::dnd::dnd(ev) {
                return true;
            }
            match ev {
                Event::Push | Event::Focus if auto && i.value() == AUTO => i.set_value(""),
                Event::Unfocus => fix(i, min, max, auto),
                _ => {}
            }
            false
        });
        let mut n = Self { i, min, max, auto };
        n.set_value(min);
        n
    }

    pub fn value(&self) -> u32 {
        let mut i = self.i.clone();
        fix(&mut i, self.min, self.max, self.auto); // a button click doesn't blur the box
        i.value().parse().unwrap_or(0) // "auto" -> 0
    }

    pub fn set_value(&mut self, v: u32) {
        self.i.set_value(&v.to_string());
        fix(&mut self.i, self.min, self.max, self.auto);
    }
}
