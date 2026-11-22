# Tablica

Program do pisania tekstów i wysyłania ich do cyfrowego rzutnika tekstów **CRT-20+** ([TUGAL](https://tugal.eu)) przez port szeregowy. Nowa nowoczesna wersja programu _Tablica Świetlna 8x20_ napisana w języku Rust. Działa na Windows 7+ i Linuksie.

<img src="tex/bg.png" width="302">

## Co nowego względem oryginału

- Usprawnienia i ogólne poprawy wygody użytkowania.
- Nowe opcje formatowanie tekstu: wyrównywanie do lewej, prawej, środka i justowanie, a także ustawianie położenia tekstu w pionie.
- Pobieranie psalmów na miesiąc (.txt lub .xml Microsoft Word 2003).
- Podgląd ukrytych znaków.
- Motywy kolorystyczne
- Aktualizacja instalacja jednym kliknięciem.

## Dostępne wersje

|                             |                          |
| --------------------------- | ------------------------ |
| `tablica-windows-10-64.exe` | Windows 10 / 11 (64-bit) |
| `tablica-windows-7-64.exe`  | Windows 7 (64-bit)       |
| `tablica-windows-7-32.exe`  | Windows 7 (32-bit)       |
| `tablica-linux-64`          | Linux (64-bit)           |
| `tablica-linux-32`          | Linux (32-bit)           |

## Budowanie ze źródła

Wymagany Rust **nightly** i [`just`](https://github.com/casey/just): `just build`, `just test`. Na Linuksie potrzebne są też pakiety FLTK i `libudev-dev` (lista w [.github/workflows/build.yml](https://raw.githubusercontent.com/eye-wave/8x20/refs/heads/main/.github/workflows/build.yml)).

## Licencja

GPL-3.0-or-later

## Podziękowania

- Krzysztof Lewiński, autor oryginalnego programu _Tablica Świetlna 8x20_, na którego protokole oparta jest komunikacja z rzutnikiem.
- Firma [TUGAL](https://tugal.eu), producent rzutnika CRT-20+.

## Licencja

GPL-3.0-or-later.
