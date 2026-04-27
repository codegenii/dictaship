use std::{env, path::Path};

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    // OUT_DIR is <target>/<profile>/build/<pkg>-<hash>/out — go up 3 levels to reach <target>/<profile>/
    let profile_dir = Path::new(&out_dir).ancestors().nth(3).unwrap();
    std::fs::copy("config.toml", profile_dir.join("config.toml"))
        .expect("failed to copy config.toml to output dir");
    println!("cargo:rerun-if-changed=config.toml");

    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Embed the application manifest.
        // Declares Common Controls v6, Windows 10/11 compatibility, PerMonitorV2 DPI awareness.
        let manifest = env::current_dir()
            .unwrap()
            .join("dictaship.exe.manifest");
        println!("cargo:rerun-if-changed=dictaship.exe.manifest");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");

        // Compile the VS_VERSIONINFO resource so right-click → Properties → Details is populated.
        // Version numbers are read from CARGO_PKG_VERSION automatically.
        let mut res = winres::WindowsResource::new();
        res.set("ProductName",      "Dictaship");
        res.set("FileDescription",  "Voice dictation with AI distillation");
        res.set("OriginalFilename", "dictaship.exe");
        res.set("LegalCopyright",   "Copyright \u{00A9} 2025 Evgenii Grebeniuk");
        res.compile().expect("failed to compile version resource");
    }
}
