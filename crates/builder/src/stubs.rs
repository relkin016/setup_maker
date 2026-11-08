//! Access to the Windows installer stubs.

static X64: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/stub_x64.bin"));
static X86: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/stub_x86.bin"));

pub fn get(arch: &str) -> Result<Vec<u8>, String> {
    let embedded = if arch == "x86" { X86 } else { X64 };
    if !embedded.is_empty() {
        return Ok(embedded.to_vec());
    }
    let name = format!("smk-stub-{}.exe", arch);
    let mut cands = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            cands.push(d.join("stubs").join(&name));
            cands.push(d.join(&name));
        }
    }
    cands.push(std::path::PathBuf::from("stubs").join(&name));
    for c in &cands {
        if let Ok(b) = std::fs::read(c) {
            return Ok(b);
        }
    }
    Err(format!("Не найден стаб установщика {} (ожидался в {:?})", name, cands))
}

