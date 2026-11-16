set quiet
set shell := ["bash", "-cue"]

target := if os_family() == "windows" { "x86_64-pc-windows-msvc" } else { "x86_64-unknown-linux-gnu" }
exe := if os_family() == "windows" { ".exe" } else { "" }
bin := "target/" + target + "/release/tablica" + exe

build:
    cargo --config .cargo/size.toml build --release --target {{ target }}

    if [ "{{ os_family() }}" != "windows" ]; then \
        strip --strip-all -R .comment "{{ bin }}" || true; \
        sstrip "{{ bin }}" 2>/dev/null || true; \
    fi
    ls -l "{{ bin }}" | awk '{printf "%.2f KiB {{ bin }}\n", $5/1024}'

test:
    cargo test --target {{ target }}
