//! Shared vanilla colour calculations.

/// `Mth.hsvToArgb`: packed ARGB, with Java's float arithmetic and truncation.
pub fn hsv_to_argb(hue: f32, saturation: f32, value: f32, alpha: u32) -> u32 {
    let h = (hue * 6.0) as i32 % 6;
    let f = hue * 6.0 - h as f32;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - f * saturation);
    let t = value * (1.0 - (1.0 - f) * saturation);
    let (r, g, b) = match h {
        0 => (value, t, p),
        1 => (q, value, p),
        2 => (p, value, t),
        3 => (p, q, value),
        4 => (t, p, value),
        5 => (value, p, q),
        _ => panic!("invalid HSV hue"),
    };
    let c = |v: f32| ((v * 255.0) as i32).clamp(0, 255) as u32;
    (alpha & 255) << 24 | c(r) << 16 | c(g) << 8 | c(b)
}
