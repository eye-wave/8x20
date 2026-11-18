use super::*;

// Folder picker; empty path = cancelled. Windows 10+ (feature "rfd"): modern dialog; otherwise (incl. Win7): fltk's.
// no parent window handed to rfd; pass fltk's HWND via set_parent if it ever opens behind the app.
pub fn pick_dir() -> PathBuf {
    #[cfg(all(windows, feature = "rfd"))]
    return rfd::FileDialog::new().pick_folder().unwrap_or_default();
    #[cfg(not(all(windows, feature = "rfd")))]
    {
        let mut d = NativeFileChooser::new(NativeFileChooserType::BrowseDir);
        d.show();
        d.filename()
    }
}

// "[#####               ] 25%"
pub fn bar(done: u32, total: u32) -> String {
    let p = (done * 100 / total.max(1)).min(100);
    let n = (p / 5) as usize;
    format!("[{}{}] {p}%", "#".repeat(n), " ".repeat(20 - n))
}

// Selected port name (index 0 is the "not selected" entry). The names are kept separately because
// FLTK's menu labels treat '/' as a submenu separator.
pub fn port_of(c: &menu::Choice, names: &[String]) -> Option<String> {
    usize::try_from(c.value() - 1)
        .ok()
        .and_then(|i| names.get(i).cloned())
}

// Digits-aware comparison: COM3 < COM10, ttyS3 < ttyS10; case-insensitive.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
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
pub fn bg<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
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

#[cfg(windows)]
pub fn taskbar(win: &window::Window) {
    use std::ffi::c_void;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetWindowLongW(h: *mut c_void, i: i32) -> i32;
        fn SetWindowLongW(h: *mut c_void, i: i32, v: i32) -> i32;
        fn ShowWindow(h: *mut c_void, c: i32) -> i32;
    }
    let h = win.raw_handle() as *mut c_void;
    unsafe {
        let s = GetWindowLongW(h, -20); // GWL_EXSTYLE
        SetWindowLongW(h, -20, (s | 0x40000) & !0x80); // + WS_EX_APPWINDOW, - WS_EX_TOOLWINDOW
        ShowWindow(h, 0); // hide + show so the taskbar notices
        ShowWindow(h, 5);
    }
}
