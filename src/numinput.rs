//! Number input without arrows: digits only, clamped to `max`; empty input snaps to `min` on blur.
//! With `auto`, 0 is shown as "auto" and a click clears it so the user can type.
use crate::theme::{Theme, skin, theme};
use fltk::{enums::*, input::Input, prelude::*};
use std::{cell::RefCell, rc::Rc};

type Cb = Rc<RefCell<Option<Box<dyn FnMut()>>>>;

fn fire(cb: &Cb) {
    if let Ok(mut c) = cb.try_borrow_mut()
        && let Some(f) = c.as_mut()
    {
        f()
    }
}

#[derive(Clone)]
pub struct NumInput {
    i: Input,
    min: u32,
    max: u32,
    auto: bool,
    cb: Cb,
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
        let cb: Cb = Rc::default();
        i.set_trigger(CallbackTrigger::Changed);
        i.set_callback({
            let cb = cb.clone();
            move |i| {
                let d: String = i.value().chars().filter(char::is_ascii_digit).collect();
                let s = match d.parse::<u64>() {
                    Ok(n) => n.min(max as u64).to_string(),
                    Err(_) if d.is_empty() => String::new(),
                    Err(_) => max.to_string(), // too many digits to even parse
                };
                if i.value() != s {
                    i.set_value(&s);
                }
                fire(&cb);
            }
        });
        i.super_handle_first(false); // so drops reach our handler before FLTK pastes them
        i.handle({
            let cb = cb.clone();
            move |i, ev| {
                if crate::dnd::dnd(ev) {
                    return true;
                }
                match ev {
                    Event::Push | Event::Focus if auto && i.value() == AUTO => i.set_value(""),
                    Event::Unfocus => {
                        fix(i, min, max, auto);
                        fire(&cb);
                    }
                    _ => {}
                }
                false
            }
        });
        let mut n = Self {
            i,
            min,
            max,
            auto,
            cb,
        };
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

    /// The number as typed right now (None while the box is empty); unlike `value()` it never rewrites the box.
    pub fn raw(&self) -> Option<u32> {
        self.i.value().parse().ok()
    }

    /// Called after every edit (and when the box loses focus).
    pub fn on_change(&self, f: impl FnMut() + 'static) {
        *self.cb.borrow_mut() = Some(Box::new(f));
    }

    /// Red background while `bad`.
    pub fn mark(&self, bad: bool) {
        let mut i = self.i.clone();
        i.set_color(if bad {
            Color::from_hex(0xE08080)
        } else {
            Color::from_hex(theme().colors.text_bg)
        });
        i.redraw();
    }

    /// The underlying widget, for adding it to a group.
    pub fn widget(&self) -> Input {
        self.i.clone()
    }
}
