use std::{env, path::Path};

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    // OUT_DIR is <target>/<profile>/build/<pkg>-<hash>/out — go up 3 levels to reach <target>/<profile>/
    let profile_dir = Path::new(&out_dir).ancestors().nth(3).unwrap();
    std::fs::copy("config.toml", profile_dir.join("config.toml"))
        .expect("failed to copy config.toml to output dir");
    println!("cargo:rerun-if-changed=config.toml");

    // Embed the application manifest (MSVC only).
    // The manifest declares Common Controls v6, Windows 10/11 compatibility,
    // and PerMonitorV2 DPI awareness.
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest = env::current_dir()
            .unwrap()
            .join("dictaphile.exe.manifest");
        println!("cargo:rerun-if-changed=dictaphile.exe.manifest");
        println!(
            "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
            manifest.display()
        );
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    }
}
