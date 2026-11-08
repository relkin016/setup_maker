//! Wizard images -> raw BGRA resource ([w u32][h u32][pixels top-down]).

use std::path::Path;

pub fn load_bgra(path: &Path, max_w: u32, max_h: u32) -> Result<Vec<u8>, String> {
    let img = image::open(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let img = if img.width() > max_w || img.height() > max_h {
        img.resize(max_w, max_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut out = Vec::with_capacity(8 + (w * h * 4) as usize);
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    for px in rgba.pixels() {
        let [r, g, b, a] = px.0;
        // pre-blend with white (the wizard background)
        let bl = |c: u8| ((c as u32 * a as u32 + 255 * (255 - a as u32)) / 255) as u8;
        out.extend_from_slice(&[bl(b), bl(g), bl(r), 255]);
    }
    Ok(out)
}
