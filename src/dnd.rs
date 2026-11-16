use super::*;

pub type OnDrop = Rc<dyn Fn(PathBuf)>;
thread_local!(pub static ON_DROP: RefCell<Option<OnDrop>> = const { RefCell::new(None) });
pub static DROP: AtomicBool = AtomicBool::new(false);

// "file:///a%20b" (X11/Wayland) or a plain path (Windows) -> path
pub fn uri_path(l: &str) -> PathBuf {
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

// While the "Więcej" overlay is up, dropped files are ignored too.
pub static MODAL: AtomicBool = AtomicBool::new(false);
