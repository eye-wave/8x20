mod themes_build;

fn main() {
    themes_build::run();

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("tex/icon.ico")
            .compile()
            .unwrap();
    }
}
