// Text engine: line rules (COLS x ROWS pages), alignment, word delete and space collapsing.
// Pure buffer logic; no widgets are created here.
use fltk::{prelude::*, *};

pub const COLS: usize = 20; // max characters per line
pub const ROWS: i32 = 8; // lines per page

// Delete the selection, else the word before (back) / after the caret; at a line edge, the newline.
pub fn kill(e: &mut text::TextEditor, back: bool) -> i32 {
    let Some(mut b) = e.buffer() else { return 0 };
    let p = e.insert_position();
    let (s, z) = match b.selection_position().filter(|(s, z)| s != z) {
        Some((s, z)) => (s.min(z), s.max(z)),
        None if back => {
            let ls = b.line_start(p);
            let t = b.text_range(ls, p).unwrap_or_default();
            let keep = t
                .trim_end_matches(' ')
                .trim_end_matches(|c: char| c != ' ')
                .len();
            (
                if p == ls {
                    (p - 1).max(0)
                } else {
                    ls + keep as i32
                },
                p,
            )
        }
        None => {
            let le = b.line_end(p);
            let t = b.text_range(p, le).unwrap_or_default();
            let rest = t
                .trim_start_matches(|c: char| c != ' ')
                .trim_start_matches(' ');
            (
                p,
                if p == le {
                    (p + 1).min(b.length())
                } else {
                    p + (t.len() - rest.len()) as i32
                },
            )
        }
    };
    b.unselect();
    b.remove(s, z);
    e.set_insert_position(s);
    e.show_insert_position();
    1
}

#[derive(Clone, Copy)]
pub enum Al {
    Left,
    Center,
    Right,
    Justify,
}

// Plain text has no alignment attribute, so alignment is real padding: a line's leading/trailing
// spaces are layout, never content, and are recomputed here. Every mode first normalizes inner space
// runs to one space (otherwise a justified line could never be left/centered/right-aligned again).
pub fn align_line(l: &str, a: Al) -> String {
    let w: Vec<&str> = l.split(' ').filter(|s| !s.is_empty()).collect();
    let t = w.join(" ");
    if t.is_empty() {
        return t; // a blank line stays blank, never padding
    }
    let room = COLS.saturating_sub(t.chars().count());
    match a {
        Al::Left => t,
        Al::Center => format!("{}{t}", " ".repeat(center_pad(&t))),
        Al::Right => format!("{}{t}", " ".repeat(room)),
        Al::Justify => {
            if w.len() < 2 {
                return t;
            }
            let gaps = w.len() - 1;
            let sp = COLS.saturating_sub(w.iter().map(|s| s.chars().count()).sum());
            let mut out = String::new();
            for (i, s) in w.iter().enumerate() {
                out += s;
                if i < gaps {
                    out += &" ".repeat(sp / gaps + (i < sp % gaps) as usize);
                }
            }
            out
        }
    }
}

// Punctuation that hangs when centering: opening marks at the start of a line and closing marks at
// its end don't count towards the line's width, so the words themselves look centered.
const OPEN: &str = "(\"'„“‘‚«[{¡¿";
const CLOSE: &str = ".,;:!?…)]}\"'”’»";

// Leading spaces that center a line by its words; hanging marks may reach into the margins but the
// line never grows past COLS. A line with nothing but punctuation is centered as is.
fn center_pad(t: &str) -> usize {
    let c: Vec<char> = t.chars().collect();
    let ld = c.iter().take_while(|x| OPEN.contains(**x)).count();
    let tr = c.iter().rev().take_while(|x| CLOSE.contains(**x)).count();
    let core = c.len().saturating_sub(ld + tr);
    if core == 0 {
        return COLS.saturating_sub(c.len()).div_ceil(2); // odd leftover space goes left
    }
    let spaces = COLS.saturating_sub(core).div_ceil(2).saturating_sub(ld);
    spaces.saturating_sub((spaces + c.len()).saturating_sub(COLS))
}

// After an edit: every whitespace-only line except the caret's own is cleared to a bare newline.
pub fn tidy(e: &text::TextEditor, b: &mut text::TextBuffer) {
    let cur = b.line_start(e.insert_position()) as usize;
    let (txt, mut p, mut blank) = (b.text(), 0, vec![]);
    for l in txt.split('\n') {
        if p != cur && !l.is_empty() && l.bytes().all(|c| c == b' ') {
            blank.push((p, p + l.len()));
        }
        p += l.len() + 1;
    }
    for (s, z) in blank.into_iter().rev() {
        b.remove(s as i32, z as i32);
    }
}

// Start offset of line n (0-based), or the buffer end if there are fewer lines.
fn line_pos(b: &text::TextBuffer, n: i32) -> i32 {
    if n == 0 {
        return 0;
    }
    b.text()
        .match_indices('\n')
        .nth(n as usize - 1)
        .map_or(b.length(), |(i, _)| i as i32 + 1)
}

// Guess how a line was aligned from its padding (alignment isn't stored anywhere).
fn infer(l: &str) -> Al {
    let t = l.trim_start_matches(' ');
    let lead = l.len() - t.len();
    let t = t.trim_end_matches(' ');
    let room = COLS.saturating_sub(t.chars().count());
    match (lead, t.contains("  ")) {
        (0, true) => Al::Justify,
        (0, false) => Al::Left,
        _ if lead >= room => Al::Right,
        _ => Al::Center,
    }
}

// A newline was deleted at pos, joining two lines: collapse the spaces of the joined line and
// re-align it the way the first line was aligned.
pub fn rejoin(e: &mut text::TextEditor, b: &mut text::TextBuffer, pos: i32) {
    if pos > b.length() {
        return;
    }
    let (ls, le) = (b.line_start(pos), b.line_end(pos));
    let line = b.text_range(ls, le).unwrap_or_default();
    let head = b.text_range(ls, pos).unwrap_or_default();
    if align_line(&line, Al::Left).chars().count() > COLS {
        return; // would not fit anyway; leave it alone
    }
    let new = align_line(&line, infer(&head));
    if new == line {
        return;
    }
    // caret goes right after the last character that was before the join
    let k = head.chars().filter(|&c| c != ' ').count();
    let off = match k {
        0 => new.len() - new.trim_start_matches(' ').len(),
        _ => new
            .char_indices()
            .filter(|&(_, c)| c != ' ')
            .nth(k - 1)
            .map_or(new.len(), |(i, c)| i + c.len_utf8()),
    };
    b.replace(ls, le, &new);
    e.set_insert_position(ls + off as i32);
    e.show_insert_position();
}

// Align the selected lines; with no selection, the whole page the caret is on.
pub fn align(e: &mut text::TextEditor, b: &mut text::TextBuffer, a: Al) {
    let p = e.insert_position();
    let row = b.count_lines(0, p); // caret stays on its row so the view doesn't jump
    let (ls, le) = match b.selection_position().filter(|(s, z)| s != z) {
        Some((s, z)) => {
            let (s, z) = (s.min(z), s.max(z));
            // selection ending at a line start doesn't include that line
            let z = if b.line_start(z) == z { z - 1 } else { z };
            (b.line_start(s), b.line_end(z))
        }
        None => {
            let first = b.count_lines(0, p) / ROWS * ROWS;
            (
                line_pos(b, first),
                b.line_end(line_pos(b, first + ROWS - 1)),
            )
        }
    };
    let new = b
        .text_range(ls, le)
        .unwrap_or_default()
        .split('\n')
        .map(|l| align_line(l, a))
        .collect::<Vec<_>>()
        .join("\n");
    b.replace(ls, le, &new);
    b.unselect();
    let ln = line_pos(b, row);
    e.set_insert_position(b.line_end(ln));
    e.show_insert_position();
}

// Typing a character at the end of a full line: the word being typed moves to the next line, if
// that is empty (or missing and allowed). A word that starts at column 0 stays put (returns None).
fn hop(b: &mut text::TextBuffer, pos: i32, ch: char, can_add_line: bool) -> Option<i32> {
    let before = b.text_range(b.line_start(pos), pos).unwrap_or_default();
    let word = before.rsplit(' ').next().unwrap_or_default();
    let head = &before[..before.len() - word.len()];
    let at_end = pos >= b.length();
    if head.is_empty()
        || !(if at_end {
            can_add_line
        } else {
            b.line_end(pos + 1) == pos + 1
        })
    {
        return None;
    }
    let from = pos - (before.len() - head.trim_end_matches(' ').len()) as i32; // also drop the gap
    let w = format!("{word}{ch}");
    b.remove(from, pos);
    if at_end {
        b.insert(from, &format!("\n{w}"));
    } else {
        b.insert(from + 1, &w);
    }
    Some(from + 1 + w.len() as i32)
}

// Insert text at the caret, enforcing COLS per line and (optionally) a max line count.
// Typing is blocked at the limit; pasting hard-wraps at the limit instead.
pub fn put(
    e: &mut text::TextEditor,
    b: &mut text::TextBuffer,
    s: &str,
    max_lines: Option<i32>,
    wrap: bool,
) {
    if let Some((a, z)) = b.selection_position() {
        if a != z {
            b.remove(a, z);
            e.set_insert_position(a);
        }
        b.unselect();
    }
    let mut pos = e.insert_position();
    for ch in s.chars() {
        let ch = if ch == '\t' { ' ' } else { ch };
        if ch == '\r' || (ch != '\n' && ch.is_control()) {
            continue;
        }
        let (ls, le) = (b.line_start(pos), b.line_end(pos));
        let len = b.text_range(ls, le).unwrap_or_default().chars().count();
        let room = max_lines.is_none_or(|m| b.count_lines(0, b.length()) + 1 < m);
        // typing a space at the end of a full line = Enter
        let ch = if !wrap && ch == ' ' && len >= COLS && pos == le {
            '\n'
        } else {
            ch
        };
        if !wrap
            && ch != ' '
            && ch != '\n'
            && len >= COLS
            && pos == le
            && let Some(np) = hop(b, pos, ch, room)
        {
            pos = np;
            continue;
        }
        let out = if ch == '\n' {
            if !room {
                continue;
            }
            "\n".to_string()
        } else if len < COLS {
            ch.to_string()
        } else if wrap && pos == le && room && ch != ' ' {
            format!("\n{ch}")
        } else {
            continue;
        };
        b.insert(pos, &out);
        pos += out.len() as i32;
    }
    e.set_insert_position(pos);
    e.show_insert_position();
}

// True if the text just inserted at pos..pos+ins left any line over COLS or too many lines.
pub fn bad(b: &text::TextBuffer, pos: i32, ins: i32, max_lines: Option<i32>) -> bool {
    let (mut p, end) = (b.line_start(pos), b.line_end((pos + ins).min(b.length())));
    loop {
        let le = b.line_end(p);
        if b.text_range(p, le).unwrap_or_default().chars().count() > COLS {
            return true;
        }
        if le >= end || le >= b.length() {
            break;
        }
        p = le + 1;
    }
    max_lines.is_some_and(|m| b.count_lines(0, b.length()) + 1 > m)
}

// Move a page's text block (first..last non-blank row) to the top or the vertical middle of ROWS
// rows. Inner blank rows are kept; an odd spare row goes below. A blank page is returned as is.
pub fn valign_page(rows: &[String], middle: bool) -> Vec<String> {
    let r = ROWS as usize;
    let used = |l: &String| !l.trim().is_empty();
    let (Some(f), Some(l)) = (rows.iter().position(used), rows.iter().rposition(used)) else {
        return rows.to_vec();
    };
    let mut v = vec![String::new(); if middle { (r - (l - f + 1)) / 2 } else { 0 }];
    v.extend_from_slice(&rows[f..=l]);
    v.resize(r, String::new());
    v
}

// Vertical alignment: with a selection, every page from the first to the last one it touches;
// with none, the page the caret is on. Pages stay exactly ROWS rows (so page boundaries don't
// shift); only the buffer's final page loses its trailing blank rows, as decode() does.
pub fn valign(e: &mut text::TextEditor, b: &mut text::TextBuffer, middle: bool) {
    let r = ROWS as usize;
    let rows: Vec<String> = b.text().split('\n').map(String::from).collect();
    let n = rows.len();
    let (s, z) = match b.selection_position().filter(|(s, z)| s != z) {
        Some((s, z)) => {
            let (s, z) = (s.min(z), s.max(z));
            (s, if b.line_start(z) == z { z - 1 } else { z }) // selection ending at a line start
        }
        None => (e.insert_position(), e.insert_position()),
    };
    let last = (n - 1) / r;
    let p0 = (b.count_lines(0, s) as usize / r).min(last);
    let p1 = (b.count_lines(0, z) as usize / r).clamp(p0, last);
    let mut out: Vec<String> = (p0..=p1)
        .flat_map(|k| {
            let page: Vec<String> = (0..r)
                .map(|i| rows.get(k * r + i).cloned().unwrap_or_default())
                .collect();
            valign_page(&page, middle)
        })
        .collect();
    let ls = line_pos(b, (p0 * r) as i32);
    let le = if (p1 + 1) * r < n {
        line_pos(b, ((p1 + 1) * r) as i32) - 1
    } else {
        let keep = (p1 - p0) * r + 1; // last page: drop its trailing blanks, never earlier padding
        while out.len() > keep && out.last().is_some_and(|l| l.is_empty()) {
            out.pop();
        }
        b.length()
    };
    let new = out.join("\n");
    // caret at the end of the last text row
    let k = out.iter().rposition(|l| !l.is_empty()).map_or(0, |k| {
        out[..=k].iter().map(|l| l.len() + 1).sum::<usize>() - 1
    });
    b.replace(ls, le, &new);
    b.unselect();
    e.set_insert_position(ls + k as i32);
    e.show_insert_position();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_stays_blank() {
        assert_eq!(align_line("   ", Al::Center), "");
    }

    #[test]
    fn alignment() {
        let sp = |n| " ".repeat(n);
        assert_eq!(align_line("abcd", Al::Center), format!("{}abcd", sp(8)));
        assert_eq!(align_line("  abcd  ", Al::Right), format!("{}abcd", sp(16)));
        assert_eq!(align_line("  abcd  ", Al::Left), "abcd");
        assert_eq!(align_line("ab cd", Al::Justify), format!("ab{}cd", sp(16)));
        assert_eq!(
            align_line(" a b c ", Al::Justify),
            format!("a{}b{}c", sp(9), sp(8))
        );
        assert_eq!(align_line("word", Al::Justify), "word");
        let j = align_line("ab cd", Al::Justify); // justified lines must be undoable
        assert_eq!(align_line(&j, Al::Left), "ab cd");
        assert_eq!(align_line(&j, Al::Center), format!("{}ab cd", sp(8)));
    }

    #[test]
    fn join_collapses_and_realigns() {
        let sp = |n| " ".repeat(n);
        assert!(matches!(infer(&format!("{}abc", sp(8))), Al::Center));
        assert!(matches!(infer(&format!("{}abc", sp(17))), Al::Right));
        assert!(matches!(infer(&format!("ab{}cd", sp(16))), Al::Justify));
        assert!(matches!(infer("ab cd"), Al::Left));
        let j = align_line(&format!("{}abcdef", sp(5)), infer(&format!("{}ab", sp(9))));
        assert_eq!(j, format!("{}abcdef", sp(7)));
    }

    #[test]
    fn vertical() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let page = v(&["", "a", "", "b", "", "", "", ""]);
        assert_eq!(
            valign_page(&page, true),
            v(&["", "", "a", "", "b", "", "", ""])
        );
        assert_eq!(
            valign_page(&page, false),
            v(&["a", "", "b", "", "", "", "", ""])
        );
        assert_eq!(valign_page(&v(&[""; 8]), true), v(&[""; 8]));
    }

    #[test]
    fn center_ignores_punctuation() {
        let sp = |n| " ".repeat(n);
        assert_eq!(
            align_line("abcdef.", Al::Center),
            format!("{}abcdef.", sp(7))
        );
        assert_eq!(align_line("abcdef", Al::Center), format!("{}abcdef", sp(7)));
        assert_eq!(
            align_line("\"abcdef\"", Al::Center),
            format!("{}\"abcdef\"", sp(6))
        );
        assert_eq!(
            align_line("abcdef!?", Al::Center),
            format!("{}abcdef!?", sp(7))
        );
        assert_eq!(align_line("...", Al::Center), format!("{}...", sp(9))); // odd leftover goes left
        assert_eq!(align_line("abc", Al::Center), format!("{}abc", sp(9)));
        assert_eq!(
            align_line("abcdefghijklmnopqrs.", Al::Center),
            "abcdefghijklmnopqrs."
        );
        assert_eq!(
            align_line("abcdef.", Al::Right),
            format!("{}abcdef.", sp(13))
        );
    }
}
