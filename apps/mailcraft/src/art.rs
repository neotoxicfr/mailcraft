// MailCraft's icon, drawn in code (original work, MIT OR Apache-2.0): a white envelope on a
// rounded teal square. Used for the window, the notification area and, through build.rs, the
// .exe and the installer.

/// RGBA pixels, `size`×`size`, anti-aliased by 4×4 supersampling.
pub fn icon_rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4;
    let teal = [0x0E_u8, 0x80, 0x74];
    let mut out = vec![0u8; (size * size * 4) as usize];
    let s = size as f32;
    for y in 0..size {
        for x in 0..size {
            let (mut bg, mut fg) = (0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = (x as f32 + (sx as f32 + 0.5) / SS as f32) / s;
                    let py = (y as f32 + (sy as f32 + 0.5) / SS as f32) / s;
                    if in_rounded_square(px, py, 0.06, 0.2) {
                        bg += 1;
                        if on_envelope(px, py, (1.6 / s).max(0.05)) {
                            fg += 1;
                        }
                    }
                }
            }
            let n = (SS * SS) as f32;
            let (a, w) = (bg as f32 / n, fg as f32 / n);
            let i = ((y * size + x) * 4) as usize;
            for c in 0..3 {
                // White over teal, by the share of samples on the envelope.
                let v = f32::from(teal[c]) * (a - w) + 255.0 * w;
                out[i + c] = if a > 0.0 {
                    (v / a).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                };
            }
            out[i + 3] = (a * 255.0).round() as u8;
        }
    }
    out
}

fn in_rounded_square(x: f32, y: f32, margin: f32, radius: f32) -> bool {
    let (lo, hi) = (margin, 1.0 - margin);
    if x < lo || x > hi || y < lo || y > hi {
        return false;
    }
    let cx = x.clamp(lo + radius, hi - radius);
    let cy = y.clamp(lo + radius, hi - radius);
    (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
}

/// Envelope outline and flap, `w` thick (in unit square coordinates).
fn on_envelope(x: f32, y: f32, w: f32) -> bool {
    let (l, r, t, b) = (0.22, 0.78, 0.32, 0.7);
    let inside = x >= l - w / 2.0 && x <= r + w / 2.0 && y >= t - w / 2.0 && y <= b + w / 2.0;
    if !inside {
        return false;
    }
    let border = x - l < w / 2.0 || r - x < w / 2.0 || y - t < w / 2.0 || b - y < w / 2.0;
    // Flap: lines from the top corners to the middle point.
    let (mx, my) = (0.5, 0.53);
    let dist = |ax: f32, ay: f32| {
        let (dx, dy) = (mx - ax, my - ay);
        let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
        ((x - ax - t * dx).powi(2) + (y - ay - t * dy).powi(2)).sqrt()
    };
    border || dist(l, t) < w / 2.0 || dist(r, t) < w / 2.0
}
