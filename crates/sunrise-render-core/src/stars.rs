//! The night sky's stars, built once from vanilla's fixed seed (`SkyRenderer.buildStars`) as star space triangles on a sphere of radius 100: up to 1500 quads, each turned to face the centre and spun about its own axis, two triangles per quad.

use alloc::vec::Vec;
use glam::Vec3;

use crate::random::{LegacyRandom, RandomSource};

pub fn star_triangles() -> Vec<[f32; 3]> {
    let mut random = LegacyRandom::new(10842);
    let mut out = Vec::with_capacity(1500 * 6);
    for _ in 0..1500 {
        let x = random.next_float() * 2.0 - 1.0;
        let y = random.next_float() * 2.0 - 1.0;
        let z = random.next_float() * 2.0 - 1.0;
        let size = 0.15 + random.next_float() * 0.1;
        let l = x * x + y * y + z * z;
        if l <= 0.010_000_001 || l >= 1.0 {
            continue;
        }
        let center = Vec3::new(x, y, z).normalize() * 100.0;
        let z_rot = (random.next_double() * core::f64::consts::TAU) as f32;
        // Matrix3f.rotateTowards(-center, up).rotateZ(-zRot).
        let forward = -center.normalize();
        let right = Vec3::Y.cross(forward).normalize_or(Vec3::X);
        let up = forward.cross(right);
        let (s, c) = (-z_rot).sin_cos();
        let (r2, u2) = (right * c + up * s, up * c - right * s);
        let corner = |a: f32, b: f32| (center + r2 * a + u2 * b).to_array();
        let quad = [
            corner(size, -size),
            corner(size, size),
            corner(-size, size),
            corner(-size, -size),
        ];
        out.extend([quad[0], quad[1], quad[2], quad[0], quad[2], quad[3]]);
    }
    out
}
