//! CRT-20+ text sign over a serial port (protocol ported from tablica.exe).
//! Frame: FF FF | cmd | len(u16 BE) | payload | crc(2)   len = 5 + payload.len()
//! One page = 8 rows x 20 bytes (Windows-1250). Pages[0] is the title page.
use serialport::{ClearBuffer, SerialPort};
use std::{
    io::{self, ErrorKind},
    time::Duration,
};

pub type Pages = Vec<Vec<Vec<u8>>>;

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

// Exact port of the exe's routine: odd 0xAA seed, then two zero bytes appended.
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

// read until idle timeout; replies' CRC unchecked.
fn rx(p: &mut dyn SerialPort) -> io::Result<Vec<u8>> {
    let (mut v, mut b) = (vec![], [0u8; 256]);
    p.set_timeout(Duration::from_secs(3))?; // first byte may be slow
    loop {
        match p.read(&mut b) {
            Ok(0) => break,
            Ok(n) => {
                v.extend_from_slice(&b[..n]);
                p.set_timeout(Duration::from_millis(300))?; // then an idle gap ends the reply
            }
            Err(e) if e.kind() == ErrorKind::TimedOut => break,
            Err(e) => return Err(e),
        }
    }
    let skip = v.iter().position(|&x| x != 0xFF).unwrap_or(v.len()); // exe drops leading FFs
    Ok(v.split_off(skip))
}

fn send(p: &mut dyn SerialPort, f: &[u8]) -> io::Result<Vec<u8>> {
    p.clear(ClearBuffer::All)?; // exe purges RX+TX before every send
    p.write_all(f)?;
    rx(p)
}

// Reply is cmd 5: [05 len len 14 a0] + 160 bytes at r[5..165].
fn page(p: &mut dyn SerialPort, f: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let r = send(p, f)?;
    if r.len() < 165 || r[0] != 5 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "brak poprawnej odpowiedzi z tablicy",
        ));
    }
    Ok((0..8)
        .map(|i| r[5 + 20 * i..25 + 20 * i].to_vec())
        .collect())
}

fn ack(r: Vec<u8>) -> io::Result<()> {
    if r.first() == Some(&6) {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::InvalidData,
            "tablica nie potwierdziła zapisu",
        ))
    }
}

// 115200 8N1, no flow control; DTR/RTS low like the exe.
fn open(name: &str) -> io::Result<Box<dyn SerialPort>> {
    let mut p = serialport::new(name, 115_200)
        .timeout(Duration::from_millis(500))
        .open()?;
    p.write_data_terminal_ready(false)?;
    p.write_request_to_send(false)?;
    std::thread::sleep(Duration::from_millis(500)); // let the adapter settle after open
    Ok(p)
}

/// Slot N: title page via cmd 1 + N, then pages 1..=pages via cmd 3 + page number.
pub fn read(port: &str, n: u16, pages: u8) -> io::Result<Pages> {
    let mut p = open(port)?;
    (0..=pages)
        .map(|k| {
            page(
                &mut *p,
                &if k == 0 {
                    frame(1, &n.to_be_bytes())
                } else {
                    frame(3, &[k])
                },
            )
        })
        .collect()
}

/// Every page as cmd 4 [page index, 8x20 bytes padded to 256] waiting for ack, then cmd 2 [N] commits the slot.
pub fn write(port: &str, n: u16, pages: &Pages) -> io::Result<()> {
    let mut p = open(port)?;
    for (k, rows) in pages.iter().enumerate() {
        let mut pl = vec![b' '; 257];
        pl[0] = k as u8;
        for (i, row) in rows.iter().take(8).enumerate() {
            pl[1 + 20 * i..1 + 20 * i + row.len()].copy_from_slice(row); // rows are <= 20 bytes (checked by caller)
        }
        ack(send(&mut *p, &frame(4, &pl))?)?;
    }
    ack(send(&mut *p, &frame(2, &n.to_be_bytes()))?)
}

pub fn ports() -> Vec<String> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .collect()
}
