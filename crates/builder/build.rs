// Embeds the prebuilt Windows stubs (paths given by SMK_STUB_X64 / SMK_STUB_X86).
// Without them the builder looks for `stubs/smk-stub-x64.exe` next to itself.
use std::{env, fs, path::PathBuf};

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    for (var, name) in [("SMK_STUB_X64", "stub_x64.bin"), ("SMK_STUB_X86", "stub_x86.bin")] {
        println!("cargo:rerun-if-env-changed={}", var);
        let dst = out.join(name);
        match env::var(var) {
            Ok(p) if !p.is_empty() => {
                println!("cargo:rerun-if-changed={}", p);
                fs::copy(&p, &dst).unwrap_or_else(|e| panic!("{}: {}", p, e));
            }
            _ => {
                fs::write(&dst, b"").unwrap();
            }
        }
    }
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // GUI subsystem is chosen at runtime (console kept for CLI use).
    }
}
