use std::{env, path::Path};

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    // OUT_DIR is <target>/<profile>/build/<pkg>-<hash>/out — go up 3 levels to reach <target>/<profile>/
    let profile_dir = Path::new(&out_dir).ancestors().nth(3).unwrap();
    std::fs::copy("config.toml", profile_dir.join("config.toml"))
        .expect("failed to copy config.toml to output dir");
    println!("cargo:rerun-if-changed=config.toml");
}
