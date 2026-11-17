use crate::theme::civil;

use super::*;
use regex_lite::{Captures, Regex};
use std::sync::atomic::{AtomicU8, AtomicU32};

// "Psalmy na miesiąc": pick year + month + one or more output formats, download, save into a folder
// "<Miesiąc> <rok>" (one file per day and format, as the old TypeScript tool did).

#[derive(Clone, Copy)]
pub enum Format {
    Word2003,
    Tablica,
}

// (format, texture name in tex/ (63x71), file extension, caption drawn on the placeholder texture)
const FORMATS: &[(Format, &str, &str, &str)] = &[
    (Format::Word2003, "fmt_word", "xml", "Word"),
    (Format::Tablica, "fmt_txt", "txt", "TXT"),
];

static FORMATS_ON: AtomicU8 = AtomicU8::new(0b11);
static BUSY: AtomicBool = AtomicBool::new(false);
static DONE: AtomicU32 = AtomicU32::new(0);
static TOTAL: AtomicU32 = AtomicU32::new(1);

const NOT_COMPLETED: &str = "Materiały nie zostały jeszcze uzupełnione";
const NOTHING_FOUND: &str = "Nie znaleziono wersów na tej stronie";
const NETWORK: &str = "Wystąpił błąd z połączeniem sieciowym";
const MONTHS: [&str; 12] = [
    "Styczeń",
    "Luty",
    "Marzec",
    "Kwiecień",
    "Maj",
    "Czerwiec",
    "Lipiec",
    "Sierpień",
    "Wrzesień",
    "Październik",
    "Listopad",
    "Grudzień",
];

#[derive(Clone)]
pub struct Verse {
    psalm: bool, // false = acclamation
    chorus: String,
    text: Vec<String>,
}

pub struct Psalm {
    year: u32,
    month: u32,
    day: u32,
    year_letter: Option<String>,
    day_type: Option<String>,
    verses: Vec<Verse>,
}

// ---- scraping (port of api/search.ts + api/scraper.ts) ----
// JS `.` stops at \r and \n, `\w` is ASCII: the patterns below keep that.
const PW: &str = "[^a-zA-ZąćęłńóśźżĄĆĘŁŃÓŚŻŹ]";
struct Re {
    year: Regex,
    day_type: Regex,
    psalm: Regex,
    acc: Regex,
    not_done: Regex,
    chorus_albo: Regex,
    chorus_end: Regex,
    strong: Regex,
    tags: Regex,
    paren: Regex,
    lone: Regex,
    nonletter: Regex,
    ws: Regex,
    wrap: Regex,
    albo: Regex,
}
static RE: LazyLock<Re> = LazyLock::new(|| {
    let r = |p: &str| Regex::new(p).unwrap();
    Re {
        year: r(r"Rok ((?-u:\w))[^\r\n]+h1>"),
        day_type: r(r#"fw-bold"><em>([^<]+)"#),
        psalm: r(r"Psalm[^\r\n]*<em>([^<]+)[^\r\n]*<p>([\s\S]*?)\s{8,}"),
        acc: r(r"Aklamacja[^\r\n]*<em>([^<]+)[^\r\n]+<p>([\s\S]*?)<strong>"),
        not_done: r(r"Materiały[^\r\n]*nie zostały uzupełnione\."),
        chorus_albo: r(&format!("{PW}+albo[^\r\n]*")),
        chorus_end: r(&format!("{PW}+$")),
        strong: r(r"<strong[^\r\n]*strong>"),
        tags: r(r"<[^<]+>"),
        paren: r(r"\([^\r\n]+"),
        lone: r(r"\s+(?-u:\w)\s+"),
        nonletter: r(PW),
        ws: r(r"\s+"),
        wrap: r(r"[^\r\n]{1,20}\s"),
        albo: r(r"albo[^\r\n]+"),
    }
});
// First match of `re` in the rest of the page; if `f` accepts it, the page is advanced to 2 bytes past
// the match start (the matches start with ASCII), exactly like the old `optimize()`.
fn find<T>(h: &mut &str, re: &Regex, f: impl FnOnce(&Captures) -> Option<T>) -> Option<T> {
    let s = *h;
    let c = re.captures(s)?;
    let r = f(&c)?;
    *h = &s[c.get(0)?.start() + 2..];
    Some(r)
}

fn trim_lines(s: &str) -> String {
    s.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

// acclamations repeating a psalm/another acclamation are dropped: compare letters only
fn norm(t: &[String]) -> String {
    let j = t.join("");
    let j = RE.lone.replace_all(&j, " ");
    RE.nonletter.replace_all(&j, "").to_lowercase()
}

fn scrape(html: &str, (year, month, day): (u32, u32, u32)) -> Result<Psalm, String> {
    if RE.not_done.is_match(html) {
        return Err(NOT_COMPLETED.into());
    }
    let mut h = html;
    let year_letter = find(&mut h, &RE.year, |c| Some(c[1].to_string()));
    let day_type = find(&mut h, &RE.day_type, |c| Some(c[1].to_string()));
    let mut verses: Vec<Verse> = vec![];
    loop {
        let mut none = true;
        let p = find(&mut h, &RE.psalm, |c| {
            let ch = RE.chorus_albo.replace(&c[1], "").into_owned();
            let chorus = RE.chorus_end.replace(&ch, "").trim().to_string();
            if chorus.is_empty() {
                return None;
            }
            let t = RE.strong.replace_all(&c[2], "").into_owned();
            let t = RE.tags.replace_all(&t, "").replace("\r\n", "\n");
            let text = t.split("\n\n").map(trim_lines).collect();
            Some(Verse {
                psalm: true,
                chorus,
                text,
            })
        });
        if let Some(p) = p {
            none = false;
            if !verses.iter().any(|v| v.chorus == p.chorus) {
                verses.push(p);
            }
        }
        let a = find(&mut h, &RE.acc, |c| {
            let chorus = RE.paren.replace(&c[1], "").trim().to_string();
            if chorus.is_empty() {
                return None;
            }
            let t = RE.tags.replace_all(&c[2], "").replace("\r\n", "\n");
            let text = t.split('\n').map(|l| l.trim().to_string()).collect();
            Some(Verse {
                psalm: false,
                chorus,
                text,
            })
        });
        if let Some(a) = a {
            none = false;
            if !verses.iter().any(|v| norm(&v.text) == norm(&a.text)) {
                verses.push(a);
            }
        }
        if none {
            break;
        }
    }
    if verses.is_empty() {
        return Err(NOTHING_FOUND.into());
    }
    Ok(Psalm {
        year,
        month,
        day,
        year_letter,
        day_type,
        verses,
    })
}

fn days_in(y: u32, m: u32) -> u32 {
    match m {
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

// Scrape the psalms of one month. Calls `progress(done, total)` as it goes (drives the bar).
// Stops at the first day whose materials are not published yet; days without verses are skipped.
// sequential, no retries; add when the site starts rate-limiting.
fn fetch(year: u32, month: u32, progress: &dyn Fn(u32, u32)) -> Result<Vec<Psalm>, String> {
    // rustls (not schannel): Windows 7 schannel has TLS 1.2 off by default
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent("Bot Izajasz")
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .into();
    let n = days_in(year, month);
    let mut out = vec![];
    for day in 1..=n {
        progress(day - 1, n);
        let url = format!("https://niezbednik.niedziela.pl/liturgia/{year}-{month:02}-{day:02}");
        let html = agent
            .get(&url)
            .call()
            .and_then(|mut r| r.body_mut().read_to_string())
            .map_err(|e| {
                popup(&e.to_string());
                NETWORK.to_string()
            })?;
        match scrape(&html, (year, month, day)) {
            Ok(p) => out.push(p),
            Err(e) if e == NOT_COMPLETED => break,
            Err(e) if e == NOTHING_FOUND => {}
            Err(e) => return Err(e),
        }
    }
    progress(n, n);
    if out.is_empty() {
        Err(NOT_COMPLETED.into())
    } else {
        Ok(out)
    }
}

// ---- TXT (port of formatters/txt) ----
// Rows are WINDOWS-1250 bytes; the page layout itself is written by file_bytes() (file.rs).
fn center(s: &str, h: usize) -> String {
    let lines: Vec<String> = s
        .split('\n')
        .map(|l| " ".repeat(20usize.saturating_sub(l.chars().count()).div_ceil(2)) + l)
        .take(h)
        .collect();
    let top = (h - lines.len()) / 2;
    let bot = h - top - lines.len();
    [vec![String::new(); top], lines, vec![String::new(); bot]]
        .concat()
        .join("\n")
}

// lines of at most 20 chars, broken at spaces
// the old "1-letter word never ends a line" step was dead code (never fired), so it is gone.
fn wrap(s: &str) -> String {
    let t = format!("{s}\n");
    let v: Vec<_> = RE
        .wrap
        .find_iter(&t)
        .map(|m| RE.ws.replace(m.as_str(), " ").trim().to_string())
        .collect();
    v.join("\n")
}

// two lines: the first one ends after 5 syllables
fn split_words(s: &str) -> String {
    let (mut n, mut idx) = (0, 0);
    for w in s.split_whitespace() {
        let c: Vec<char> = w.to_uppercase().chars().collect();
        let vowels = c.iter().filter(|&&x| "AĄEĘIOÓUY".contains(x)).count();
        let (mut ia, mut i) = (0, 0);
        while i + 1 < c.len() {
            if c[i] == 'I' && "AEĄĘUO".contains(c[i + 1]) {
                ia += 1;
                i += 2;
            } else {
                i += 1;
            }
        }
        n += vowels - ia;
        if n >= 5 {
            idx = s.find(w).unwrap() + w.len();
            break;
        }
    }
    format!("{}\n{}", s[..idx].trim(), s[idx..].trim())
}

fn txt(p: &Psalm) -> Vec<u8> {
    let date = format!("{:02}.{:02}.{}", p.day, p.month, p.year);
    let rok = p
        .year_letter
        .as_ref()
        .map_or(String::new(), |l| format!("Rok {l}"));
    let day_type = p
        .day_type
        .as_deref()
        .map_or(String::new(), |d| RE.albo.replace(d, "").into_owned());
    let first = [
        format!("   TEKST NR {:03}", p.day),
        "PSALM RESPONSORYJNY".into(),
        center(&format!("{date} {rok}"), 1),
        String::new(),
        center(&wrap(&day_type), 4),
    ]
    .join("\n");
    let pages: sign::Pages = std::iter::once(first)
        .chain(p.verses.iter().map(|v| {
            center(
                &wrap(&split_words(&format!("{}.", v.chorus).to_uppercase())),
                8,
            )
        }))
        .map(|pg| {
            pg.split('\n')
                .map(|r| WINDOWS_1250.encode(r).0.into_owned())
                .collect()
        })
        .collect();
    file_bytes(&pages)
}

// ---- Word 2003 XML (port of formatters/xml) ----
// the old code joined JS arrays with template strings, which left stray "," between the
// paragraphs; they are not emitted here (valid WordML without them).
fn run(text: &str, color: &str) -> String {
    format!(
        r#"<w:r><w:rPr><w:color w:val="{color}" /><w:sz w:val="36" /></w:rPr><w:t>{text}</w:t></w:r>"#
    )
}

fn chorus_p(t: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:rPr><w:sz w:val="36" /></w:rPr></w:pPr>{}<w:r><w:rPr><w:b /><w:sz w:val="36" /></w:rPr><w:t>{t}</w:t></w:r></w:p>"#,
        run("Refren: ", "FF0000")
    )
}

fn lines_p<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let v: Vec<&str> = lines.collect();
    v.iter()
        .enumerate()
        .map(|(i, l)| {
            let star = l.ends_with(['*', '†']);
            let (text, mark) = match l.chars().last() {
                Some(c) if star => (&l[..l.len() - c.len_utf8()], run(&c.to_string(), "FF0000")),
                _ => (*l, String::new()),
            };
            let gap = if i + 1 == v.len() { "" } else { "<w:contextualSpacing />" };
            format!(
                r#"<w:p><w:pPr>{gap}<w:rPr><w:sz w:val="36" /></w:rPr></w:pPr><w:r><w:rPr><w:sz w:val="36" /></w:rPr><w:tab /><w:t>{}</w:t></w:r>{mark}</w:p>"#,
                text.trim()
            )
        })
        .collect()
}

fn word(p: &Psalm) -> String {
    let page_break = if p.verses.len() > 3 {
        r#"<w:br w:type="page"/>"#
    } else {
        ""
    };
    let verses: String = p
        .verses
        .iter()
        .map(|v| {
            let mut s = format!(
                "<w:p>{}</w:p>",
                run(
                    if v.psalm {
                        "PSALM RESPONSORYJNY"
                    } else {
                        "AKLAMACJA PRZED EWANGELIĄ"
                    },
                    "FF0000"
                )
            );
            if v.psalm {
                for st in &v.text {
                    s += &(chorus_p(&v.chorus) + &lines_p(st.split('\n')));
                }
            } else {
                s += &(chorus_p(&v.chorus) + &lines_p(v.text.iter().flat_map(|t| t.split('\n'))));
            }
            s + &chorus_p(&v.chorus) + page_break
        })
        .collect();
    let head = format!(
        "<w:p>{}</w:p>",
        run(
            &format!(
                "{} (rok {})",
                p.day_type.as_deref().unwrap_or(""),
                p.year_letter.as_deref().unwrap_or("")
            ),
            "1F497D"
        )
    );
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<?mso-application progid="Word.Document"?>
<w:wordDocument xmlns:aml="http://schemas.microsoft.com/aml/2001/core" xmlns:o="urn:schemas-microsoft-com:office:office"
  xmlns:v="urn:schemas-microsoft-com:vml" xmlns:w="http://schemas.microsoft.com/office/word/2003/wordml"
  xmlns:wx="http://schemas.microsoft.com/office/word/2003/auxHint"
  xmlns:wsp="http://schemas.microsoft.com/office/word/2003/wordml/sp2" xml:space="preserve">
  <w:fonts><w:font w:name="Times New Roman" /></w:fonts>
  <w:styles>
    <w:style w:type="paragraph" w:default="on" w:styleId="Normalny">
      <w:name w:val="Normal" />
      <wx:uiName wx:val="Normalny" />
      <w:pPr><w:spacing w:after="200" w:line="276" w:line-rule="auto" /></w:pPr>
    </w:style>
  </w:styles>
  <w:docPr><w:view w:val="print" /></w:docPr>
  <w:body>
    {head}
    {verses}
    <w:sectPr>
      <w:pgSz w:w="11906" w:h="16838" />
      <w:pgMar w:top="709" w:right="1417" w:bottom="709" w:left="1417" w:header="708" w:footer="708" w:gutter="0" />
      <w:cols w:space="708" />
      <w:docGrid w:line-pitch="360" />
    </w:sectPr>
  </w:body>
</w:wordDocument>"#
    )
}

fn render(p: &Psalm, f: Format) -> Vec<u8> {
    match f {
        Format::Word2003 => word(p).into_bytes(),
        Format::Tablica => txt(p),
    }
}

// Ask for a folder (UI thread), write "<Miesiąc> <rok>/NNN.<ext>" there.
fn save(files: Vec<(String, Vec<u8>)>, dir_name: &str) -> Result<usize, String> {
    let mut d = NativeFileChooser::new(NativeFileChooserType::BrowseDir);
    d.show();
    let base = d.filename();
    if base.as_os_str().is_empty() {
        return Err("anulowano".into());
    }
    let dir = base.join(dir_name);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (n, b) in &files {
        fs::write(dir.join(n), b).map_err(|e| e.to_string())?;
    }
    Ok(files.len())
}

fn progress(done: u32, total: u32) {
    DONE.store(done, Relaxed);
    TOTAL.store(total, Relaxed);
}

fn start(year: u32, month: u32, status: &Frame) {
    if BUSY.swap(true, Relaxed) {
        return;
    }
    progress(0, 1);
    let mut st = status.clone();
    st.set_label_font(FONT); // monospace so the bar doesn't jiggle
    st.set_label(&bar(0, 1));
    let mut tick = st.clone();
    app::add_timeout3(0.1, move |h| {
        if BUSY.load(Relaxed) {
            tick.set_label(&bar(DONE.load(Relaxed), TOTAL.load(Relaxed)));
            app::repeat_timeout3(0.1, h);
        }
    });
    let on = FORMATS_ON.load(Relaxed);
    bg(
        move || {
            let ps = fetch(year, month, &progress)?;
            let mut files = vec![];
            for p in &ps {
                for (i, &(f, _, ext, _)) in FORMATS.iter().enumerate() {
                    if on >> i & 1 == 1 {
                        files.push((format!("{:03}.{ext}", p.day), render(p, f)));
                    }
                }
            }
            Ok::<_, String>(files)
        },
        move |r| {
            BUSY.store(false, Relaxed);
            st.set_label_font(Font::Helvetica);
            let dir = format!("{} {year}", MONTHS[month as usize - 1]);
            st.set_label(&match r.and_then(|f| save(f, &dir)) {
                Ok(n) => format!("Zapisano plików: {n}"),
                Err(e) => format!("Błąd: {e}"),
            });
        },
    );
}

fn paint_marks(marks: &mut [Frame], t: &Theme) {
    let sel = FORMATS_ON.load(Relaxed);
    for (i, m) in marks.iter_mut().enumerate() {
        let c = if sel >> i & 1 == 1 {
            t.colors.accent
        } else {
            t.colors.panel
        };
        m.set_color(Color::from_hex(c));
    }
}

fn paint(marks: &mut [Frame], status: &mut Frame, panel: &mut Frame, t: &Theme) {
    paint_marks(marks, t);
    panel.set_color(Color::from_hex(t.colors.panel));
    status.set_color(Color::from_hex(t.colors.panel));
    status.set_label_color(Color::from_hex(t.colors.scrim));
}

// Create it hidden, inside the root group, like the other overlays.
pub fn psalms_overlay(ed: &text::TextEditor) -> group::Group {
    let (w, h) = (460, 348);
    let (ox, oy) = ((960 - w) / 2, (728 - h) / 2);
    let mut g = group::Group::new(0, 0, 960, 728, None);
    scrim();
    let mut panel = Frame::new(ox, oy, w, h, None);
    panel.set_frame(FrameType::BorderBox);
    let mut title = lbl("Psalmy na miesiąc", ox + 16, oy + 12, w - 32, 28, 20);
    title.set_label_font(Font::HelveticaBold);

    // year + month, default: next month
    let (y, m, _) = civil();
    let (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    lbl("Rok:", ox + 16, oy + 56, 40, 24, 14);
    let mut year = NumInput::new(ox + 60, oy + 56, 2000, 2100, false);
    year.set_value(y as u32);
    lbl("Miesiąc:", ox + 140, oy + 56, 70, 24, 14);
    let mut month = NumInput::new(ox + 214, oy + 56, 1, 12, false);
    month.set_value(m);

    // format: 63x71 image toggle buttons; each selected one sits on an accent-coloured mat
    lbl("Formaty:", ox + 16, oy + 96, 120, 24, 14);
    let mut marks: Vec<Frame> = (0..FORMATS.len() as i32)
        .map(|i| {
            let mut f = Frame::new(ox + 16 + i * 84, oy + 122, 71, 79, None);
            f.set_frame(FrameType::FlatBox);
            f
        })
        .collect();
    for (i, &(_, tex, _, caption)) in FORMATS.iter().enumerate() {
        let mut b = btn(tex, ox + 20 + i as i32 * 84, oy + 126);
        b.set_label(caption); // placeholder: drop these 3 lines once the textures carry their own text
        b.set_label_size(11);
        b.set_label_color(Color::from_hex(theme().colors.scrim));
        let mut marks = marks.clone();
        b.set_callback(move |b| {
            // toggle; at least one format stays selected
            let now = FORMATS_ON.load(Relaxed) ^ (1 << i);
            if now != 0 {
                FORMATS_ON.store(now, Relaxed);
            }
            paint_marks(&mut marks, theme());
            if let Some(mut g) = b.parent() {
                g.redraw(); // the mats sit under the buttons
            }
        });
    }

    // download button: 428x44 (tex/pobierz_{off,on}.svg) + the progress bar / result line
    let mut dl = btn("pobierz", ox + 16, oy + 217);
    let mut status = lbl("", ox + 16, oy + 273, w - 32, 24, 13);
    status.set_frame(FrameType::FlatBox);
    dl.set_callback({
        let status = status.clone();
        move |_| start(year.value(), month.value(), &status)
    });

    let mut close = button::Button::new(ox + w - 116, oy + h - 44, 100, 28, "Zamknij");
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

    paint(&mut marks, &mut status, &mut panel, theme());
    skin(move |t| paint(&mut marks, &mut status, &mut panel, t));
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrape_and_render() {
        let html = "<h1>Rok C</h1>\n<p class=\"fw-bold\"><em>Niedziela albo coś</em>\n<b>Psalm</b> <em>Pan jest moim pasterzem, nie brak mi niczego albo inaczej.</em><p>Pan jest\nmoim pasterzem\n\nNa zielonych pastwiskach\n        x";
        let p = scrape(html, (2026, 12, 1)).unwrap();
        assert_eq!(p.year_letter.as_deref(), Some("C"));
        assert_eq!(
            p.verses[0].chorus,
            "Pan jest moim pasterzem, nie brak mi niczego"
        );
        assert_eq!(p.verses[0].text.len(), 2);
        let t = txt(&p);
        assert!(t.starts_with(file::DECO));
        assert!(word(&p).contains("Refren: "));
        assert_eq!(
            scrape("Materiały nie zostały uzupełnione.", (2026, 12, 1))
                .err()
                .as_deref(),
            Some(NOT_COMPLETED)
        );
    }
}
