// Compiles res/stub.rc (manifest, default icon, version) with mingw windres.
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=res");
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let obj = out.join("stub_res.o");
    let windres = if target.starts_with("x86_64") {
        "x86_64-w64-mingw32-windres"
    } else {
        "i686-w64-mingw32-windres"
    };
    let windres = env::var("WINDRES").unwrap_or_else(|_| windres.to_string());
    let st = Command::new(&windres)
        .args(["--input-format=rc", "-O", "coff", "-c", "65001", "-i", "res/stub.rc", "-o"])
        .arg(&obj)
        .status()
        .expect("windres not found (install mingw-w64)");
    assert!(st.success(), "windres failed");
    println!("cargo:rustc-link-arg={}", obj.display());
}
