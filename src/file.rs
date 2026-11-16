use super::*;

// Load a file into the editors and remember it as the current file.
pub fn open_file(
    f: &Path,
    b: &mut text::TextBuffer,
    tb: &mut text::TextBuffer,
    path: &RefCell<Option<PathBuf>>,
    msg: &mut Frame,
) {
    match fs::read(f)
        .map_err(|e| e.to_string())
        .and_then(|t| load(&t))
    {
        Ok(l) => {
            tb.set_text(&l.title);
            b.set_text(&l.body);
            *path.borrow_mut() = Some(f.to_path_buf());
            msg.set_label(&if l.soft + l.hard > 0 {
                format!(
                    "Otwarto; zawinięto linie: miękko {}, twardo {}",
                    l.soft, l.hard
                )
            } else {
                "Otwarto".to_string()
            });
        }
        Err(e) => popup(&format!("Nie można otworzyć: {e}")),
    }
}

pub const DECO: &[u8] = b"<------------------>";

// File format: every page (title page = 000 first) is DECO / "<- Strona nr: NNN ->" / DECO
// followed by exactly ROWS rows of at most COLS bytes (Windows-1250, CRLF).
pub fn file_bytes(pages: &sign::Pages) -> Vec<u8> {
    let mut out = vec![];
    for (k, rows) in pages.iter().enumerate() {
        out.extend_from_slice(DECO);
        out.extend_from_slice(format!("\r\n<- Strona nr: {k:03} ->\r\n").as_bytes());
        out.extend_from_slice(DECO);
        out.extend_from_slice(b"\r\n");
        for i in 0..ROWS as usize {
            out.extend_from_slice(rows.get(i).map_or(&[][..], |r| &r[..]));
            out.extend_from_slice(b"\r\n");
        }
    }
    out
}

// A decoration line such as <------------------> (only '<', '-' and '>' characters, at least one arrow).
pub fn is_deco(l: &str) -> bool {
    let t = l.trim();
    t.contains(['<', '>']) && t.chars().all(|c| matches!(c, '<' | '-' | '>'))
}

// "<- Strona nr: 007 ->" -> Some(7); tolerant of spacing and of a missing closing arrow.
pub fn page_header(l: &str) -> Option<usize> {
    let d = l.trim().strip_prefix("<- Strona nr:")?.trim_start();
    d.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

// Fit a line into COLS: soft wrap at the last space that fits (indent kept, the spaces at the break
// dropped); a word longer than a line is hard-split at COLS. Tabs become spaces, other control
// characters are dropped, trailing spaces are layout and removed.
pub fn wrap_line(l: &str, soft: &mut usize, hard: &mut usize) -> Vec<String> {
    let mut c: Vec<char> = l
        .chars()
        .map(|c| if c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect();
    while c.last() == Some(&' ') {
        c.pop();
    }
    let mut out = vec![];
    while c.len() > COLS {
        let lead = c.iter().take_while(|&&x| x == ' ').count();
        match (lead + 1..=COLS).rev().find(|&i| c[i] == ' ') {
            Some(i) => {
                out.push(c[..i].iter().collect::<String>().trim_end().to_string());
                let k = i + c[i..].iter().take_while(|&&x| x == ' ').count();
                c.drain(..k);
                *soft += 1;
            }
            None => {
                out.push(c[..COLS].iter().collect());
                c.drain(..COLS);
                *hard += 1;
            }
        }
    }
    out.push(c.into_iter().collect());
    out
}

pub struct Loaded {
    pub title: String,
    pub body: String,
    pub soft: usize, // lines that were soft wrapped
    pub hard: usize, // lines that were hard wrapped
}

// Load a text file with or without the page decoration (DECO / "<- Strona nr: NNN ->" / DECO):
// - decoration lines are optional and ignored; a "<- Strona nr: N ->" line alone starts page N;
// - rows before the first header belong to the title page (page 0);
// - a file with no header at all is plain text: all of it is the body, the title stays default;
// - UTF-8 is accepted as well as Windows-1250;
// - lines over COLS are soft/hard wrapped; a page that grows past ROWS rows spills onto an extra
//   page (the following pages move down so no text is lost or merged).
pub fn load(bytes: &[u8]) -> Result<Loaded, String> {
    let text = match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => WINDOWS_1250.decode(bytes).0.into_owned(),
    };
    let text = text
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last().is_some_and(|x| x.is_empty()) {
        lines.pop();
    }
    let (mut soft, mut hard) = (0, 0);
    let mut pages: Vec<Vec<String>> = vec![vec![]];
    let (mut cur, mut headed) = (0usize, false);
    for l in lines {
        if is_deco(l) {
            continue;
        }
        if let Some(n) = page_header(l) {
            if n > 127 {
                return Err(format!("Strona {n} za wysoka (max 127)"));
            }
            (cur, headed) = (n, true);
            if pages.len() <= n {
                pages.resize(n + 1, vec![]);
            }
            continue;
        }
        pages[cur].extend(wrap_line(l, &mut soft, &mut hard));
    }
    let title = if headed {
        let keep = pages[0].len().min(ROWS as usize);
        let extra = pages[0].split_off(keep);
        if !extra.is_empty() {
            pages.insert(1, extra); // title rows beyond ROWS get a body page of their own
        }
        let mut t = pages[0].clone();
        while t.last().is_some_and(|x| x.is_empty()) {
            t.pop();
        }
        t.join("\n")
    } else {
        let all = std::mem::take(&mut pages[0]);
        pages.push(all);
        default_title()
    };
    let mut rows: Vec<String> = vec![];
    for p in &pages[1..] {
        let n = p.len().div_ceil(ROWS as usize).max(1) * ROWS as usize;
        rows.extend(p.iter().cloned());
        rows.resize(rows.len() + n - p.len(), String::new()); // pad to whole pages
    }
    while rows.last().is_some_and(|x| x.is_empty()) {
        rows.pop();
    }
    Ok(Loaded {
        title,
        body: rows.join("\n"),
        soft,
        hard,
    })
}

// Sign rows (Windows-1250, space padded) -> editor text; trailing blank rows dropped.
pub fn decode(rows: &[Vec<u8>]) -> String {
    let mut v: Vec<String> = rows
        .iter()
        .map(|r| {
            WINDOWS_1250
                .decode(r)
                .0
                .trim_end_matches([' ', '\0'])
                .to_string()
        })
        .collect();
    while v.last().is_some_and(|l| l.is_empty()) {
        v.pop();
    }
    v.join("\n")
}

// Editor text (UTF-8) -> Windows-1250 bytes. A character the code page lacks becomes '?' (counted in
// `replaced`); it is never an error. Zero-width characters are invisible, so they are just dropped.
pub fn cp1250(l: &str, replaced: &mut usize) -> Vec<u8> {
    l.chars()
        .filter(|c| !matches!(c, '\u{200b}'..='\u{200d}' | '\u{feff}'))
        .map(|c| {
            let mut buf = [0u8; 4];
            let (b, _, bad) = WINDOWS_1250.encode(c.encode_utf8(&mut buf));
            if bad {
                *replaced += 1;
                b'?'
            } else {
                b[0]
            }
        })
        .collect()
}

// Editor text -> pages (title page first, then the body in chunks of ROWS lines) as Windows-1250,
// plus how many characters had to be replaced. Used by both save and send.
pub fn to_pages(title: &str, body: &str) -> Result<(sign::Pages, usize), String> {
    let mut replaced = 0;
    let mut rows = |t: &str| -> Result<Vec<Vec<u8>>, String> {
        t.lines()
            .map(|l| {
                let b = cp1250(l, &mut replaced);
                if b.len() > COLS {
                    Err(format!("Linia ma więcej niż {COLS} znaków: {l}"))
                } else {
                    Ok(b)
                }
            })
            .collect()
    };
    let mut pages = vec![rows(title)?];
    if pages[0].len() > ROWS as usize {
        return Err(format!("Strona tytułowa ma więcej niż {ROWS} linii"));
    }
    pages.extend(rows(body)?.chunks(ROWS as usize).map(|c| c.to_vec()));
    if pages.len() > 128 {
        return Err("Za dużo stron (max 127)".into());
    }
    Ok((pages, replaced))
}
