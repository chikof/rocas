fn main() {
    // Re-run this script if the icon changes.
    println!("cargo:rerun-if-changed=assets/rocas.ico");

    // Embed the Windows icon into the PE binary.
    // Only runs when cross-compiling or building natively on Windows.
    // On Linux/macOS this block is skipped entirely.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/rocas.ico");
        res.compile()
            .expect("failed to compile Windows resources");
    }
}
