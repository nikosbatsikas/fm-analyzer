//! Embeds the application icon and version info into the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "FM Analyzer");
        res.set("FileDescription", "FM Analyzer - FM broadcast analyzer for RTL-SDR");
        res.set("CompanyName", "Nikos Batsikas");
        res.set("LegalCopyright", "Copyright (c) 2026 Nikos Batsikas");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Windows icon: {e}");
        }
    }
}
