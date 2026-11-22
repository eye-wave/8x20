use super::*;
use std::cell::RefCell;

// "Przetwarzanie wsadowe": two tabs.
//  Wyślij   - pick many .txt files, give each a slot number (no duplicates), send them one after another.
//  Odczytaj - read a range of slots from the sign into NNN.txt files in a chosen folder.
// Buttons are still plain FLTK (no styling) except the three texture buttons taken from the navbar.

static CANCEL: AtomicBool = AtomicBool::new(false);
static PROGRESS: Progress = Progress::new();

type Rows = Rc<RefCell<Vec<(PathBuf, NumInput)>>>;

fn name_of(f: &Path) -> String {
    f.file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned())
}

// "007.txt" -> 7 (digits only, at most 3 of them)
fn num_of(f: &Path) -> Option<u32> {
    let s = f.file_stem()?.to_str()?;
    let txt = f.extension()?.eq_ignore_ascii_case("txt");
    (txt && s.len() <= 3 && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok())?
}

// Every file called DDD.txt -> its own number; otherwise 1, 2, 3...
fn auto_numbers(files: &[PathBuf]) -> Vec<u32> {
    files
        .iter()
        .map(|f| num_of(f))
        .collect::<Option<Vec<_>>>()
        .unwrap_or_else(|| (1..=files.len() as u32).collect())
}

// Red background on every box whose number is used twice; true if there is any such pair.
fn mark(rows: &[(PathBuf, NumInput)]) -> bool {
    let v: Vec<Option<u32>> = rows.iter().map(|r| r.1.raw()).collect();
    let mut any = false;
    for (n, x) in rows.iter().map(|r| &r.1).zip(&v) {
        let dup = x.is_some() && v.iter().filter(|y| *y == x).count() > 1;
        any |= dup;
        n.mark(dup);
    }
    any
}

// Rebuild the list: one row (file name + number box) per file, 28px apart.
fn fill(inner: &mut group::Group, sc: &mut group::Scroll, rows: &Rows, files: Vec<PathBuf>) {
    sc.scroll_to(0, 0);
    inner.clear();
    let mut r = rows.borrow_mut();
    r.clear();
    let (x, y0) = (sc.x(), sc.y());
    inner.resize(x, y0, 400, files.len() as i32 * 28);
    for (i, (f, n)) in files.iter().zip(auto_numbers(&files)).enumerate() {
        let y = y0 + i as i32 * 28;
        let mut l = Frame::new(x + 4, y, 306, 24, None);
        l.set_label(&name_of(f));
        l.set_label_size(13);
        l.set_align(Align::Left | Align::Inside | Align::Clip);
        l.set_tooltip(&f.to_string_lossy());
        let mut b = NumInput::new(x + 316, y, 0, 999, false);
        b.set_value(n);
        b.on_change({
            let rows = rows.clone();
            move || {
                mark(&rows.borrow());
            }
        });
        inner.add(&l);
        inner.add(&b.widget());
        r.push((f.clone(), b));
    }
    mark(&r);
    sc.redraw();
}

// Run a serial job on a worker thread with the progress bar on `st`; `done` makes the final message.
fn launch<T: Send + 'static>(
    busy: &Rc<Cell<bool>>,
    st: &Frame,
    set_dot: &Rc<dyn Fn(bool)>,
    port: String,
    total: u32,
    job: impl FnOnce() -> Result<T, String> + Send + 'static,
    done: impl FnOnce(T) -> String + 'static,
) {
    if busy.replace(true) {
        return;
    }
    CANCEL.store(false, Relaxed);
    PROGRESS.set(0, total);
    let mut st = st.clone();
    st.set_label_font(FONT); // monospace so the bar doesn't jiggle
    st.set_label(&bar(0, total));
    let (mut tick, b) = (st.clone(), busy.clone());
    app::add_timeout3(0.1, move |h| {
        if b.get() {
            tick.set_label(&PROGRESS.label());
            app::repeat_timeout3(0.1, h);
        }
    });
    let (busy, set_dot) = (busy.clone(), set_dot.clone());
    bg(job, move |r| {
        busy.set(false);
        st.set_label_font(Font::Helvetica);
        set_dot(r.is_ok());
        if r.is_ok() {
            cfg_set("port", &port);
        }
        st.set_label(&match r {
            Ok(v) => done(v),
            Err(e) => format!("Błąd: {e}"),
        });
    });
}

// Interrupt: the flag stops the batch between slots, sign::cancel() stops the slot in progress.
fn stop_cb(busy: Rc<Cell<bool>>, mut st: Frame) -> impl FnMut(&mut button::Button) {
    move |_| {
        if busy.get() {
            CANCEL.store(true, Relaxed);
            sign::cancel();
            st.set_label("Przerywanie...");
        }
    }
}

pub fn batch_overlay(
    ed: &text::TextEditor,
    port: &menu::Choice,
    ports: &Rc<Vec<String>>,
    set_dot: &Rc<dyn Fn(bool)>,
    busy: &Rc<Cell<bool>>,
) -> group::Group {
    let (w, h) = (640, 600);
    let (ox, oy) = ((960 - w) / 2, (728 - h) / 2);
    let mut g = group::Group::new(0, 0, 960, 728, None);
    scrim();
    let mut panel = Frame::new(ox, oy, w, h, None);
    panel.set_frame(FrameType::BorderBox);
    let mut title = lbl("Przetwarzanie wsadowe", ox + 16, oy + 12, w - 32, 28, 20);
    title.set_label_font(Font::HelveticaBold);

    let mut t_send = button::RadioButton::new(ox + 16, oy + 52, 120, 28, "Wyślij");
    let mut t_read = button::RadioButton::new(ox + 144, oy + 52, 120, 28, "Odczytaj");
    t_send.set_value(true);
    let mut status = lbl("", ox + 16, oy + h - 66, w - 32, 24, 13);
    status.set_frame(FrameType::FlatBox);

    // ---- tab "Wyślij" ----
    let rows: Rows = Rows::default();
    let sg = group::Group::new(ox, oy + 92, w, 470, None);
    let mut pick = button::Button::new(ox + 16, oy + 96, 190, 28, "Wybierz pliki .txt...");
    let mut sc = group::Scroll::new(ox + 16, oy + 132, 420, 312, None);
    sc.set_type(group::ScrollType::Vertical);
    sc.set_frame(FrameType::BorderBox);
    let inner = group::Group::new(ox + 16, oy + 132, 400, 0, None);
    inner.end();
    sc.end();
    lbl("Pierwszy numer:", ox + 452, oy + 132, 170, 24, 14);
    let mut start = NumInput::new(ox + 452, oy + 160, 0, 999, false);
    start.set_value(1);
    let mut renum = button::Button::new(ox + 452, oy + 192, 172, 28, "Numeruj od tego");
    let mut send = tool_btn("bg_lime", "icon_wyslij", "Wyślij", ox + 16, oy + 456);
    let mut stop_s = tool_btn("bg_orange", "icon_przerwij", "Przerwij", ox + 84, oy + 456);
    sg.end();

    // ---- tab "Odczytaj" ----
    let mut rg = group::Group::new(ox, oy + 92, w, 440, None);
    lbl("Od numeru:", ox + 16, oy + 104, 100, 24, 14);
    let mut from = NumInput::new(ox + 120, oy + 104, 0, 999, false);
    from.set_value(1);
    lbl("Do numeru:", ox + 200, oy + 104, 100, 24, 14);
    let mut to = NumInput::new(ox + 304, oy + 104, 0, 999, false);
    to.set_value(1);
    lbl(
        "Każdy numer zostanie zapisany jako NNN.txt w wybranym folderze.",
        ox + 16,
        oy + 140,
        w - 32,
        24,
        13,
    );
    let mut read = tool_btn("bg_rose", "icon_odczytaj", "Odczytaj", ox + 16, oy + 456);
    let mut stop_r = tool_btn("bg_orange", "icon_przerwij", "Przerwij", ox + 84, oy + 456);
    rg.end();
    rg.hide();

    let mut close = button::Button::new(ox + w - 116, oy + h - 40, 100, 28, "Zamknij");
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

    // tabs
    for (t, show, hide) in [(&mut t_send, &sg, &rg), (&mut t_read, &rg, &sg)] {
        let (mut show, mut hide) = (show.clone(), hide.clone());
        t.set_callback(move |_| {
            show.show();
            hide.hide();
            if let Some(mut p) = show.parent() {
                p.redraw();
            }
        });
    }
    stop_s.set_callback(stop_cb(busy.clone(), status.clone()));
    stop_r.set_callback(stop_cb(busy.clone(), status.clone()));

    // pick files
    pick.set_callback({
        let (mut inner, mut sc, rows) = (inner.clone(), sc.clone(), rows.clone());
        move |_| {
            let mut d = NativeFileChooser::new(NativeFileChooserType::BrowseMultiFile);
            d.set_filter("Pliki tekstowe\t*.txt");
            d.show();
            let mut files = d.filenames();
            if !files.is_empty() {
                files.sort_by(|a, b| natural_cmp(&name_of(a), &name_of(b)));
                fill(&mut inner, &mut sc, &rows, files);
            }
        }
    });
    // first number override: this one, then +1 for each next row
    renum.set_callback({
        let rows = rows.clone();
        move |_| {
            let s = start.value();
            for (i, (_, n)) in rows.borrow_mut().iter_mut().enumerate() {
                n.set_value(s + i as u32);
            }
            mark(&rows.borrow());
        }
    });

    // send
    let (port, ports, set_dot, busy) = (port.clone(), ports.clone(), set_dot.clone(), busy.clone());
    send.set_callback({
        let (rows, mut st, port, ports, set_dot, busy) = (
            rows.clone(),
            status.clone(),
            port.clone(),
            ports.clone(),
            set_dot.clone(),
            busy.clone(),
        );
        move |_| {
            if busy.get() {
                return;
            }
            let r = rows.borrow();
            if r.is_empty() {
                return st.set_label("Wybierz pliki");
            }
            r.iter().for_each(|(_, n)| {
                n.value(); // an empty box becomes 0 now, so the duplicate check sees what will be sent
            });
            if mark(&r) {
                return st.set_label("Numery nie mogą się powtarzać");
            }
            let Some(name) = port_of(&port, &ports) else {
                set_dot(false);
                return st.set_label("Wybierz port szeregowy");
            };
            // read + validate everything first, so a bad file fails before anything is sent
            let mut jobs = vec![];
            for (f, n) in r.iter() {
                let pages = fs::read(f)
                    .map_err(|e| e.to_string())
                    .and_then(|b| load(&b))
                    .and_then(|l| to_pages(&l.title, &l.body));
                match pages {
                    Ok((p, _)) => jobs.push((name_of(f), n.value() as u16, p)),
                    Err(e) => return st.set_label(&format!("{}: {e}", name_of(f))),
                }
            }
            drop(r);

            let mut ns: Vec<u16> = jobs.iter().map(|j| j.1).collect();
            ns.sort();
            let list = if ns.len() > 1 && ns.windows(2).all(|w| w[1] == w[0] + 1) {
                format!("{:03}-{:03}", ns[0], ns[ns.len() - 1]) // no gaps: a plain range
            } else {
                let j = ns
                    .iter()
                    .take(8)
                    .map(|n| format!("{n:03}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{j}{}", if ns.len() > 8 { ", ..." } else { "" })
            };
            let q = format!("Nadpisać w tablicy {} slotów?\n{list}", ns.len());
            if ask(&q, &["Nie", "Tak"]) != Some(1) {
                return;
            }

            let total = jobs.len() as u32;
            let p = name.clone();
            // the port is reopened for every file (~0.5 s each); keep it open if that gets slow
            launch(
                &busy,
                &st,
                &set_dot,
                name,
                total,
                move || {
                    for (k, (file, n, pages)) in jobs.iter().enumerate() {
                        if CANCEL.load(Relaxed) {
                            return Err("przerwano".into());
                        }
                        sign::write(&p, *n, pages).map_err(|e| format!("{file}: {e}"))?;
                        PROGRESS.done(k as u32 + 1);
                    }
                    Ok(jobs.len())
                },
                |n| format!("Wysłano plików: {n}"),
            );
        }
    });

    // read
    read.set_callback({
        let (st, port, ports, set_dot, busy) = (
            status.clone(),
            port.clone(),
            ports.clone(),
            set_dot.clone(),
            busy.clone(),
        );
        let mut st2 = status.clone();
        move |_| {
            if busy.get() {
                return;
            }
            let (a, b) = (from.value(), to.value());
            if a > b {
                return st2.set_label("Zakres jest odwrócony");
            }
            let Some(name) = port_of(&port, &ports) else {
                set_dot(false);
                return st2.set_label("Wybierz port szeregowy");
            };
            let dir = pick_dir();
            if dir.as_os_str().is_empty() {
                return;
            }
            let p = name.clone();
            launch(
                &busy,
                &st,
                &set_dot,
                name,
                b - a + 1,
                move || {
                    for n in a..=b {
                        if CANCEL.load(Relaxed) {
                            return Err("przerwano".into());
                        }
                        let pages =
                            sign::read(&p, n as u16, 0).map_err(|e| format!("{n:03}: {e}"))?;
                        fs::write(dir.join(format!("{n:03}.txt")), file_bytes(&pages))
                            .map_err(|e| e.to_string())?;
                        PROGRESS.done(n - a + 1);
                    }
                    Ok(b - a + 1)
                },
                |n| format!("Odczytano plików: {n}"),
            );
        }
    });

    let paint = |panel: &mut Frame, status: &mut Frame, sc: &mut group::Scroll, t: &Theme| {
        panel.set_color(Color::from_hex(t.colors.panel));
        status.set_color(Color::from_hex(t.colors.panel));
        status.set_label_color(Color::from_hex(t.colors.scrim));
        sc.set_color(Color::from_hex(t.colors.panel));
        panel.redraw();
    };
    paint(&mut panel, &mut status, &mut sc, theme());
    skin(move |t| paint(&mut panel, &mut status, &mut sc, t));
    g
}
