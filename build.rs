fn main() {
    // Bake today's date into the binary for the About tab.
    // Falls back to "unknown" if PowerShell isn't available (e.g. a Linux CI box).
    let date = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", "Get-Date -Format 'yyyy-MM-dd'"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_DATE={date}");

    // Embed the app icon into the Windows exe so it isn't a blank icon in
    // Explorer / the taskbar. Non-fatal if the resource compiler is unavailable.
    // Cargo.toml: [build-dependencies] winresource = "0.1"
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Failed to embed exe icon: {e}");
        }
    }
}
