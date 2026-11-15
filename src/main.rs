//! CRT-20+ text sign over COM, ported from tablica.exe (Delphi/CPort).
//! Frame: FF FF | cmd | len(u16 BE) | payload | crc(2)   len = 5 + payload.len()
//! Usage: tablica COM3 read N [PAGES [FILE.txt]] | tablica COM3 write N FILE.txt
use serialport::{ClearBuffer, SerialPort};
use std::{
    io::{self, ErrorKind},
    time::Duration,
};

// CRC-16 poly 0x8005, MSB-first table (identical to the tables in the exe).
const T: [u16; 256] = {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u16) << 8;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000 != 0 {
                (c << 1) ^ 0x8005
            } else {
                c << 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

// Exact port of the exe's routine at 0x498fcc: odd 0xAA seed, then two zero bytes appended.
fn crc(data: &[u8]) -> [u8; 2] {
    let (mut h, mut l) = (0xAAu8 ^ (T[0xAA] >> 8) as u8, 0xAAu8 ^ T[0xAA] as u8);
    for &b in data.iter().chain(&[0u8, 0]) {
        let t = T[h as usize];
        (h, l) = (l ^ (t >> 8) as u8, b ^ t as u8);
    }
    [h, l]
}

// Windows-1250 Polish letters; everything else non-ASCII becomes '?'.
// ponytail: Polish only, swap in encoding_rs for other 1250 characters.
const PL: [(char, u8); 18] = [
    ('ą', 0xB9),
    ('ć', 0xE6),
    ('ę', 0xEA),
    ('ł', 0xB3),
    ('ń', 0xF1),
    ('ó', 0xF3),
    ('ś', 0x9C),
    ('ź', 0x9F),
    ('ż', 0xBF),
    ('Ą', 0xA5),
    ('Ć', 0xC6),
    ('Ę', 0xCA),
    ('Ł', 0xA3),
    ('Ń', 0xD1),
    ('Ó', 0xD3),
    ('Ś', 0x8C),
    ('Ź', 0x8F),
    ('Ż', 0xAF),
];
fn enc(c: char) -> u8 {
    if c.is_ascii() {
        c as u8
    } else {
        PL.iter().find(|p| p.0 == c).map_or(b'?', |p| p.1)
    }
}
fn dec(b: u8) -> char {
    if b.is_ascii() {
        b as char
    } else {
        PL.iter().find(|p| p.1 == b).map_or('?', |p| p.0)
    }
}

fn frame(cmd: u8, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![cmd];
    body.extend((5 + payload.len() as u16).to_be_bytes());
    body.extend_from_slice(payload);
    let c = crc(&body);
    let mut out = vec![0xFF, 0xFF];
    out.extend(body);
    out.extend(c);
    out
}

// ponytail: read until idle timeout; the exe counts bytes but its length rule looks off by one. Replies' CRC unchecked.
fn rx(p: &mut dyn SerialPort) -> io::Result<Vec<u8>> {
    let (mut v, mut b) = (vec![], [0u8; 256]);
    p.set_timeout(Duration::from_secs(3))?; // first byte may be slow
    loop {
        match p.read(&mut b) {
            Ok(0) => break,
            Ok(n) => {
                v.extend_from_slice(&b[..n]);
                p.set_timeout(Duration::from_millis(300))?; // then idle gap ends the reply
            }
            Err(e) if e.kind() == ErrorKind::TimedOut => break,
            Err(e) => return Err(e),
        }
    }
    eprintln!("rx {v:02x?}");
    let skip = v.iter().position(|&x| x != 0xFF).unwrap_or(v.len()); // exe drops leading FFs
    Ok(v.split_off(skip))
}

fn send(p: &mut dyn SerialPort, f: &[u8]) -> io::Result<Vec<u8>> {
    p.clear(ClearBuffer::All)?; // exe purges RX+TX before every send
    eprintln!("tx {f:02x?}");
    p.write_all(f)?;
    rx(p)
}

// One page = 8 rows x 20 chars. Reply is cmd 5: [05 len len 14 a0] + 160 chars at r[5..165].
fn page(p: &mut dyn SerialPort, f: &[u8]) -> io::Result<Vec<String>> {
    let r = send(p, f)?;
    if r.len() < 165 || r[0] != 5 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!("bad reply: {r:02x?}"),
        ));
    }
    Ok((0..8)
        .map(|i| r[5 + 20 * i..25 + 20 * i].iter().map(|&b| dec(b)).collect())
        .collect())
}

const DECO: &str = "<------------------>";

// txt format of the exe: page 0 (title page) first with no header, every later page
// preceded by DECO / "<- Strona nr: 001 ->" / DECO; 8 rows of 20 chars per page.
// Slot N = page 0 via cmd 1 + N, pages 1..=pages via cmd 3 + page number (exe's read loop).
fn read_slot(p: &mut dyn SerialPort, n: u16, pages: u8) -> io::Result<String> {
    let mut out = String::new();
    for k in 0..=pages {
        let f = if k == 0 {
            frame(1, &n.to_be_bytes())
        } else {
            frame(3, &[k])
        };
        if k > 0 {
            out += &format!("{DECO}\r\n<- Strona nr: {k:03} ->\r\n{DECO}\r\n");
        }
        for row in page(p, &f)? {
            out += &row;
            out += "\r\n";
        }
    }
    Ok(out)
}

// Parse that txt format back into pages. Decorator numbers pick the page, rows before the first one are page 0.
// Rows over 20 chars or pages over 8 rows are errors, not silent truncation.
fn parse(txt: &str) -> Result<Vec<Vec<String>>, String> {
    let l: Vec<&str> = txt.lines().collect();
    let (mut pages, mut cur, mut i) = (vec![vec![]], 0usize, 0);
    while i < l.len() {
        let hdr = l[i] == DECO
            && l.get(i + 2) == Some(&DECO)
            && l.get(i + 1)
                .is_some_and(|s| s.starts_with("<- Strona nr: "));
        if hdr {
            cur = l[i + 1]
                .get(14..17)
                .and_then(|d| d.parse().ok())
                .ok_or(format!("bad page header, line {}", i + 2))?;
            if cur > 127 {
                return Err(format!("page {cur} too high (max 127)"));
            }
            pages.resize(pages.len().max(cur + 1), vec![]);
            i += 3;
            continue;
        }
        if l[i].chars().count() > 20 {
            return Err(format!("line {} is longer than 20 chars", i + 1));
        }
        pages[cur].push(l[i].to_string());
        if pages[cur].len() > 8 {
            return Err(format!("page {cur} has more than 8 rows (line {})", i + 1));
        }
        i += 1;
    }
    Ok(pages)
}

fn ack(r: Vec<u8>) -> io::Result<()> {
    if r.first() == Some(&6) {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::InvalidData,
            format!("no ack: {r:02x?}"),
        ))
    }
}

// Exe's write sequence: every page as cmd 4 [page index, 8x20 chars padded to 256] waiting for ack (cmd 6),
// then cmd 2 [N] commits the slot. pages[0] is the title page.
// ponytail: from disassembly only, never run against the sign - try a spare slot and read it back.
fn write_slot(p: &mut dyn SerialPort, n: u16, pages: &[Vec<String>]) -> io::Result<()> {
    for (k, rows) in pages.iter().enumerate() {
        let mut pl = vec![b' '; 257];
        pl[0] = k as u8;
        for (i, row) in rows.iter().take(8).enumerate() {
            for (c, ch) in row.chars().take(20).enumerate() {
                pl[1 + 20 * i + c] = enc(ch);
            }
        }
        ack(send(p, &frame(4, &pl))?)?;
    }
    ack(send(p, &frame(2, &n.to_be_bytes()))?)
}

fn main() -> io::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 {
        eprintln!("usage: tablica COMx read N [PAGES [FILE]] | write N FILE");
        std::process::exit(2);
    }
    // 115200 8N1, no flow control = the form's stored defaults
    let mut p = serialport::new(&a[1], 115_200)
        .timeout(Duration::from_millis(500))
        .open()?;
    // exe leaves DTR/RTS low (CPort dtrDisable/rtsDisable); try DTR=1 / RTS=1 env vars if silent
    let on = |k: &str| std::env::var(k).is_ok_and(|v| v == "1");
    p.write_data_terminal_ready(on("DTR"))?;
    p.write_request_to_send(on("RTS"))?;
    std::thread::sleep(Duration::from_millis(500)); // let the adapter settle after open
    let n: u16 = a[3].parse().expect("text number");
    match a[2].as_str() {
        "read" => {
            let t = read_slot(
                &mut *p,
                n,
                a.get(4).map_or(0, |s| s.parse().expect("pages")),
            )?;
            print!("{t}");
            if let Some(f) = a.get(5) {
                std::fs::write(f, t.chars().map(enc).collect::<Vec<u8>>())?; // file is Windows-1250 like the exe's
            }
        }
        "write" => {
            let txt: String = std::fs::read(a.get(4).expect("file"))?
                .iter()
                .map(|&b| dec(b))
                .collect();
            let pages = parse(&txt).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            write_slot(&mut *p, n, &pages)?;
            println!("written {} pages", pages.len());
        }
        _ => eprintln!("read|write"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames() {
        assert_eq!(frame(1, &[0, 1]), [0xff, 0xff, 1, 0, 7, 0, 1, 0xb3, 0xd1]);
        assert_eq!(frame(4, &[0; 257]).len(), 264); // exe sends 0x108 bytes
    }
    #[test]
    fn txt() {
        let t = format!("a\nb\n{DECO}\n<- Strona nr: 002 ->\n{DECO}\nc\n");
        let p = parse(&t).unwrap();
        assert_eq!(
            (p.len(), p[0].clone(), p[1].is_empty(), p[2].clone()),
            (3, vec!["a".into(), "b".into()], true, vec!["c".into()])
        );
        assert!(parse("123456789012345678901").is_err());
    }
    #[test]
    fn polish() {
        assert_eq!(enc('ł'), 0xB3);
        assert!("ąćęłńóśźżĄĆĘŁŃÓŚŹŻ".chars().all(|c| dec(enc(c)) == c));
    }
}
