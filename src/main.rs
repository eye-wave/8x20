//! CRT-20+ text sign over COM, ported from tablica.exe (Delphi/CPort).
//! Frame: FF FF | cmd | len(u16 BE) | payload | crc(2)   len = 5 + payload.len()
//! Usage: tablica scan
//!        tablica COM3 read N [PAGES [FILE.txt]]   (no FILE = raw bytes to stdout)
//!        tablica COM3 write N FILE.txt
//! Text is never converted: file bytes (Windows-1250) go to the sign and back unchanged.
//! DEBUG=1 prints tx/rx frames; DTR=1 / RTS=1 raise those lines (the exe leaves both low).
use serialport::{ClearBuffer, SerialPort};
use std::{
    io::{self, ErrorKind, Write},
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

fn debug() -> bool {
    std::env::var_os("DEBUG").is_some()
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
    if debug() {
        eprintln!("rx {v:02x?}");
    }
    let skip = v.iter().position(|&x| x != 0xFF).unwrap_or(v.len()); // exe drops leading FFs
    Ok(v.split_off(skip))
}

fn send(p: &mut dyn SerialPort, f: &[u8]) -> io::Result<Vec<u8>> {
    p.clear(ClearBuffer::All)?; // exe purges RX+TX before every send
    if debug() {
        eprintln!("tx {f:02x?}");
    }
    p.write_all(f)?;
    rx(p)
}

// One page = 8 rows x 20 bytes. Reply is cmd 5: [05 len len 14 a0] + 160 bytes at r[5..165].
fn page(p: &mut dyn SerialPort, f: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let r = send(p, f)?;
    if r.len() < 165 || r[0] != 5 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!("bad reply: {r:02x?}"),
        ));
    }
    Ok((0..8)
        .map(|i| r[5 + 20 * i..25 + 20 * i].to_vec())
        .collect())
}

const DECO: &[u8] = b"<------------------>";

// txt format of the exe: page 0 (title page) first with no header, every later page
// preceded by DECO / "<- Strona nr: 001 ->" / DECO; 8 rows of 20 bytes per page.
// Slot N = page 0 via cmd 1 + N, pages 1..=pages via cmd 3 + page number (exe's read loop).
fn read_slot(p: &mut dyn SerialPort, n: u16, pages: u8) -> io::Result<Vec<u8>> {
    let mut out = vec![];
    for k in 0..=pages {
        let f = if k == 0 {
            frame(1, &n.to_be_bytes())
        } else {
            frame(3, &[k])
        };
        if k > 0 {
            out.extend_from_slice(DECO);
            out.extend_from_slice(format!("\r\n<- Strona nr: {k:03} ->\r\n").as_bytes());
            out.extend_from_slice(DECO);
            out.extend_from_slice(b"\r\n");
        }
        for row in page(p, &f)? {
            out.extend(row);
            out.extend_from_slice(b"\r\n");
        }
    }
    Ok(out)
}

// Parse that txt format back into pages. Decorator numbers pick the page, rows before the first one are page 0.
// Rows over 20 bytes or pages over 8 rows are errors, not silent truncation.
fn parse(txt: &[u8]) -> Result<Vec<Vec<Vec<u8>>>, String> {
    let mut l: Vec<&[u8]> = txt
        .split(|&b| b == b'\n')
        .map(|x| x.strip_suffix(b"\r").unwrap_or(x))
        .collect();
    if l.last().is_some_and(|x| x.is_empty()) {
        l.pop(); // newline at end of file
    }
    let (mut pages, mut cur, mut i) = (vec![vec![]], 0usize, 0);
    while i < l.len() {
        let hdr = l[i] == DECO
            && l.get(i + 2) == Some(&DECO)
            && l.get(i + 1)
                .is_some_and(|s| s.starts_with(b"<- Strona nr: "));
        if hdr {
            cur = l[i + 1]
                .get(14..17)
                .and_then(|d| std::str::from_utf8(d).ok())
                .and_then(|d| d.parse().ok())
                .ok_or(format!("bad page header, line {}", i + 2))?;
            if cur > 127 {
                return Err(format!("page {cur} too high (max 127)"));
            }
            pages.resize(pages.len().max(cur + 1), vec![]);
            i += 3;
            continue;
        }
        if l[i].len() > 20 {
            return Err(format!("line {} is longer than 20 characters", i + 1));
        }
        pages[cur].push(l[i].to_vec());
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

// Exe's write sequence: every page as cmd 4 [page index, 8x20 bytes padded to 256] waiting for ack (cmd 6),
// then cmd 2 [N] commits the slot. pages[0] is the title page.
fn write_slot(p: &mut dyn SerialPort, n: u16, pages: &[Vec<Vec<u8>>]) -> io::Result<()> {
    for (k, rows) in pages.iter().enumerate() {
        let mut pl = vec![b' '; 257];
        pl[0] = k as u8;
        for (i, row) in rows.iter().take(8).enumerate() {
            pl[1 + 20 * i..1 + 20 * i + row.len()].copy_from_slice(row);
        }
        ack(send(p, &frame(4, &pl))?)?;
    }
    ack(send(p, &frame(2, &n.to_be_bytes()))?)
}

// 115200 8N1, no flow control = the form's stored defaults; DTR/RTS low like the exe.
fn open(name: &str) -> serialport::Result<Box<dyn SerialPort>> {
    let mut p = serialport::new(name, 115_200)
        .timeout(Duration::from_millis(500))
        .open()?;
    let on = |k: &str| std::env::var(k).is_ok_and(|v| v == "1");
    p.write_data_terminal_ready(on("DTR"))?;
    p.write_request_to_send(on("RTS"))?;
    std::thread::sleep(Duration::from_millis(500)); // let the adapter settle after open
    Ok(p)
}

// Probe every port with a read of slot 1 (changes nothing on the sign); a valid page back = the sign.
// ponytail: sequential, ~3s per silent port; ports held by another program are skipped (DEBUG=1 shows why).
fn scan() -> io::Result<()> {
    let mut found = false;
    for info in serialport::available_ports()? {
        let hit = open(&info.port_name)
            .map_err(|e| e.to_string())
            .and_then(|mut p| {
                page(&mut *p, &frame(1, &1u16.to_be_bytes())).map_err(|e| e.to_string())
            });
        match hit {
            Ok(_) => {
                println!("{}", info.port_name);
                found = true;
            }
            Err(e) if debug() => eprintln!("{}: {e}", info.port_name),
            Err(_) => {}
        }
    }
    if !found {
        eprintln!("no device answered (close tablica.exe, check the cable)");
        std::process::exit(1);
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    if a.get(1).map(String::as_str) == Some("scan") {
        return scan();
    }
    if a.len() < 4 {
        eprintln!(
            "usage: tablica scan | tablica COMx read N [PAGES [FILE]] | tablica COMx write N FILE"
        );
        std::process::exit(2);
    }
    let mut p = open(&a[1])?;
    let n: u16 = a[3].parse().expect("text number");
    match a[2].as_str() {
        "read" => {
            let t = read_slot(
                &mut *p,
                n,
                a.get(4).map_or(0, |s| s.parse().expect("pages")),
            )?;
            match a.get(5) {
                Some(f) => std::fs::write(f, &t)?,
                None => io::stdout().write_all(&t)?,
            }
        }
        "write" => {
            let txt = std::fs::read(a.get(4).expect("file"))?;
            let pages = parse(&txt).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            write_slot(&mut *p, n, &pages)?;
            println!("written {} pages", pages.len());
        }
        _ => eprintln!("read|write"),
    }
    Ok(())
}
