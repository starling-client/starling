// Copyright (c) 2014, Intel Corporation
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are met:
//
//     * Redistributions of source code must retain the above copyright notice,
//     * this list of conditions and the following disclaimer.
//
//     * Redistributions in binary form must reproduce the above copyright notice,
//     * this list of conditions and the following disclaimer in the documentation
//     * and/or other materials provided with the distribution.
//
//     * Neither the name of Intel Corporation nor the names of its contributors
//     * may be used to endorse or promote products derived from this software
//     * without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
// ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR
// ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
// LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
// ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
// SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! `Math.sin`, `Math.cos` and `Math.tan` as HotSpot computes them on x86-64: a Rust translation of Intel's libm
//! (`s_sin.S`, `s_cos.S` and `s_tan.S` from bionic, tag android-9.0.0_r1), the algorithms of the stubs that the
//! interpreter, C1 and C2 all call.
//!
//! Each floating-point operation is the original's, a single rounded `f64` operation in the original's order, with
//! no fused multiply-add. The kernels are the stubs' SSE2 sequences: where the original works on both halves of a
//! register at once (`mulpd`, `addpd`, `subpd`), so does the translation, through the `safe_arch` crate's safe
//! functions for the SSE2 intrinsics. SSE2 is part of the x86-64 baseline, so this module is built for x86-64 only
//! and has no `unsafe`; the tables are the stubs' words as `f64` arrays, aligned like the originals.
//!
//! sin and cos: x = N pi/32 + r with |r| <= pi/64, then sin(B + r) or cos(B + r) from a table of B = (N mod 64) pi/32
//! and two short polynomials (sin r - r and cos r - 1), for 2^-252 <= |x| < 90112. tan: x = N pi/32 + r likewise,
//! with a degree-15 polynomial for each of 32 table entries and, near the poles, a reciprocal of the distance to the
//! nearest one, for 0.1015625 <= |x| < 12800. Every other argument goes to a cold function with the stubs' other paths:
//! larger arguments are first reduced modulo pi/4 (pi/2 for tan) with 224 bits of 2/pi, then go through the same
//! kernel; smaller ones take a short path of their own.

use safe_arch::{
    add_m128d, add_m128d_s, bitand_m128d, bitor_m128d, convert_i32_replace_m128d_s,
    convert_to_m128d_from_lower2_i32_m128i, div_m128d_s, get_f64_from_m128d_s, load_f64_m128d_s,
    load_unaligned_m128d, m128d, mul_m128d, mul_m128d_s, set_m128d, set_m128d_s, set_splat_m128d,
    sub_m128d, sub_m128d_s, truncate_m128d_to_m128i, truncate_to_i32_m128d_s, unpack_high_m128d,
    unpack_low_m128d, zeroed_m128d,
};

/// 32/pi.
const THIRTY_TWO_OVER_PI: f64 = f64::from_bits(0x4024_5f30_6dc9_c883);
/// 2/pi.
const TWO_OVER_PI: f64 = f64::from_bits(0x3fe4_5f30_6dc9_c883);
const TWO_POW_55: f64 = f64::from_bits(0x4360_0000_0000_0000);
const TWO_POW_M55: f64 = f64::from_bits(0x3c80_0000_0000_0000);

/// pi/32 = P1 + P2 + P3 (sin and cos): P1 and P2 have at most 32 significant bits, so `N * P1` and `N * P2` are exact.
const P1: f64 = f64::from_bits(0x3fb9_21fb_5440_0000);
const P2: f64 = f64::from_bits(0x3d90_b461_1a60_0000);
const P3: f64 = f64::from_bits(0x3b63_198a_2e03_7073);

/// The polynomials sin r - r = r^3 (s3 + s5 r^2 + r^4 (s7 + s9 r^2)) and cos r - 1 = r^2 (c2 + c4 r^2 + r^4 (c6 +
/// c8 r^2)) side by side, low and high half, as the stub computes them: Taylor's coefficients 1/n! with alternating
/// signs.
const SC_1: [f64; 2] = [
    f64::from_bits(0xbfc5_5555_5555_5555),
    f64::from_bits(0xbfe0_0000_0000_0000),
];
const SC_2: [f64; 2] = [
    f64::from_bits(0x3f81_1111_1111_1111),
    f64::from_bits(0x3fa5_5555_5555_5555),
];
const SC_3: [f64; 2] = [
    f64::from_bits(0xbf2a_01a0_1a01_a01a),
    f64::from_bits(0xbf56_c16c_16c1_6c17),
];
const SC_4: [f64; 2] = [
    f64::from_bits(0x3ec7_1de3_a556_c734),
    f64::from_bits(0x3efa_01a0_1a01_a01a),
];

/// pi/32 = TP1 + TP2 + TP3 to two splits: the low half for 16 N2 (N2 = x 2/pi rounded), the high half for N.
const TP1: [f64; 2] = [
    f64::from_bits(0x3fb9_21fb_5444_4000),
    f64::from_bits(0x3fb9_21fb_5444_0000),
];
const TP2: [f64; 2] = [
    f64::from_bits(0xbd32_e7b9_6767_4000),
    f64::from_bits(0x3d46_8c23_4c4c_0000),
];
const TP3: [f64; 2] = [
    f64::from_bits(0x3aa8_a2e0_3707_344a),
    f64::from_bits(0x3ae9_8a2e_0370_7345),
];
/// About -TP2's low half: a0 + QQ_2 k0 is x - 16 N2 pi/32, the reciprocal's argument.
const QQ_2: f64 = f64::from_bits(0x3d32_e7b9_6767_33af);
/// Keeps a double's sign, exponent and top 34 significand bits.
const MASK_35: u64 = 0xffff_ffff_fffc_0000;
/// tan x - x = x^3 (Q3 + Q5 x^2 + ... + Q11 x^8) for 2^-29 <= |x| < 0.1015625.
const Q3: f64 = f64::from_bits(0x3fd5_5555_5555_5612);
const Q5: f64 = f64::from_bits(0x3fc1_1111_1109_2746);
const Q7: f64 = f64::from_bits(0x3fab_a1ba_52c4_c8ab);
const Q9: f64 = f64::from_bits(0x3f96_64a0_bf84_7a43);
const Q11: f64 = f64::from_bits(0x3f82_609a_b8fe_4d77);

/// pi/4 = PI_4_HI + PI_4_LO (sin and cos).
const PI_4_HI: f64 = f64::from_bits(0x3fe9_21fb_4000_0000);
const PI_4_LO: f64 = f64::from_bits(0x3e64_442d_1846_9899);
/// pi/4 = TAN_PI_4_HI + TAN_PI_4_LO (tan).
const TAN_PI_4_HI: f64 = f64::from_bits(0x3fe9_21fb_0000_0000);
const TAN_PI_4_LO: f64 = f64::from_bits(0x3e85_110b_4611_a626);

/// A table's words as `f64`s, 16-byte aligned like the stubs' tables, so that a pair of words is one vector load.
#[repr(C, align(16))]
struct Table<const N: usize, const M: usize>([[f64; N]; M]);

impl<const N: usize, const M: usize> Table<N, M> {
    const fn from_bits(bits: &[[u64; N]; M]) -> Self {
        let mut rows = [[0.0; N]; M];
        let mut j = 0;
        while j < M {
            let mut i = 0;
            while i < N {
                rows[j][i] = f64::from_bits(bits[j][i]);
                i += 1;
            }
            j += 1;
        }
        Table(rows)
    }
}

static SIN_COS: Table<4, 64> = Table::from_bits(&SIN_COS_TABLE);
static TAN: Table<22, 32> = Table::from_bits(&TAN_TABLE);

/// Words `i` and `i + 1` of a table row as one vector (low, high).
#[inline(always)]
fn pair<const N: usize>(row: &[f64; N], i: usize) -> m128d {
    load_unaligned_m128d(row[i..i + 2].try_into().unwrap())
}

/// The top 16 bits of `x` without the sign, which the stubs sort their argument by.
fn top(x: f64) -> u32 {
    (x.to_bits() >> 48) as u32 & 0x7fff
}

/// `Math.sin(x)` on x86-64 HotSpot.
#[inline]
pub fn sin(x: f64) -> f64 {
    if (0x3030..=0x40f5).contains(&top(x)) {
        return sin_kernel(x, 0.0, 0);
    }
    sin_far(x)
}

/// `Math.cos(x)` on x86-64 HotSpot.
#[inline]
pub fn cos(x: f64) -> f64 {
    if (0x3030..=0x40f5).contains(&top(x)) {
        return sin_kernel(x, 0.0, 16);
    }
    cos_far(x)
}

/// `Math.tan(x)` on x86-64 HotSpot.
#[inline]
pub fn tan(x: f64) -> f64 {
    if (0x3fba..=0x40c8).contains(&top(x)) {
        return tan_kernel(x, -0.0, 0);
    }
    tan_far(x)
}

#[cold]
#[inline(never)]
fn sin_far(x: f64) -> f64 {
    let top = top(x);
    if top < 0x3030 {
        // |x| < 2^-252: x (the stub's x (1 - 2^-53) for zeros and subnormals rounds back to x).
        return x;
    }
    if top >= 0x7ff0 {
        // Infinities and NaN.
        return x * -0.0;
    }
    let (hi, lo, eighths) = reduce(x, 3, PI_4_HI, PI_4_LO);
    sin_kernel(hi, lo, 8 * eighths)
}

#[cold]
#[inline(never)]
fn cos_far(x: f64) -> f64 {
    let top = top(x);
    if top < 0x3030 {
        return 1.0 - x.abs();
    }
    if top >= 0x7ff0 {
        return x * -0.0;
    }
    let (hi, lo, eighths) = reduce(x, 3, PI_4_HI, PI_4_LO);
    sin_kernel(hi, lo, 8 * eighths + 16)
}

#[cold]
#[inline(never)]
fn tan_far(x: f64) -> f64 {
    let top = top(x);
    if top < 0x3fba {
        if top < 0x0010 {
            // Zeros and subnormals.
            return x;
        }
        if top < 0x3e20 {
            // |x| < 2^-29.
            return (x + TWO_POW_55 * x) * TWO_POW_M55;
        }
        let x2 = x * x;
        let x3 = x * x2;
        let p = (((Q11 * x2 + Q9) * x2 + Q7) * x2 + Q5) * x2 + Q3;
        return x + p * x3;
    }
    if top >= 0x7ff0 {
        return x * -0.0;
    }
    let (hi, lo, quarters) = reduce(x, 2, TAN_PI_4_HI, TAN_PI_4_LO);
    tan_kernel(hi, lo, quarters)
}

/// sin(x - lo) where x = (N + steps) pi/32 + r, N = x 32/pi rounded half away from zero: `steps` 0 gives sin, 16
/// gives cos. `lo` is the low part of a reduced argument, or 0. Each value is in a vector's low half unless said
/// otherwise; a `_s` operation works on the low half only.
#[inline(always)]
fn sin_kernel(x: f64, lo: f64, steps: i32) -> f64 {
    let xs = set_m128d_s(x);
    let half = bitor_m128d(
        set_splat_m128d(0.5),
        bitand_m128d(m128d::from_bits([1 << 63, 0]), xs),
    );
    let n = truncate_to_i32_m128d_s(add_m128d_s(
        mul_m128d_s(set_m128d_s(THIRTY_TWO_OVER_PI), xs),
        half,
    ));
    let row = &SIN_COS.0[(n.wrapping_add(steps) & 63) as usize];
    let [c_hl, s_hi, s_lo, sigma] = row;
    let nf = convert_i32_replace_m128d_s(zeroed_m128d(), n);
    let nn = unpack_low_m128d(nf, nf);
    // r = x - N P1 - N P2 in both halves; minus_c = N P3 - (r1 - r - N P2) - lo is minus what r misses.
    let m1 = mul_m128d_s(set_m128d_s(P1), nf);
    let m2 = mul_m128d(set_splat_m128d(P2), nn);
    let m3 = mul_m128d_s(nn, set_m128d_s(P3));
    let r1 = sub_m128d_s(xs, m1);
    let r1r1 = unpack_low_m128d(r1, r1);
    let r = sub_m128d_s(r1, m2);
    let rr = sub_m128d(r1r1, m2);
    let minus_c = sub_m128d_s(
        sub_m128d_s(m3, sub_m128d_s(sub_m128d_s(r1r1, r), m2)),
        set_m128d_s(lo),
    );
    let r2 = mul_m128d(rr, rr);
    let r4 = mul_m128d(r2, r2);
    // (cos B, s_hi), with cos B = c_hl + sigma.
    let cos_b = add_m128d_s(pair(row, 0), load_f64_m128d_s(sigma));
    // corr = s_lo + c (cos B - s_hi r).
    let corr = add_m128d_s(
        mul_m128d(
            minus_c,
            sub_m128d_s(mul_m128d_s(load_f64_m128d_s(s_hi), r), cos_b),
        ),
        load_f64_m128d_s(s_lo),
    );
    // (sin r - r) cos B and (cos r - 1) sin B, side by side.
    let pol = add_m128d(
        add_m128d(
            mul_m128d(m128d::from_array(SC_2), r2),
            m128d::from_array(SC_1),
        ),
        mul_m128d(
            add_m128d(
                mul_m128d(mul_m128d(m128d::from_array(SC_4), r1r1), rr),
                m128d::from_array(SC_3),
            ),
            r4,
        ),
    );
    let pol = mul_m128d(pol, mul_m128d(mul_m128d_s(cos_b, r), r2));
    // s_hi + sigma r + c_hl r, as a compensated sum.
    let rsig = mul_m128d_s(load_f64_m128d_s(sigma), r);
    let med = mul_m128d_s(r, load_f64_m128d_s(c_hl));
    let res_int = add_m128d_s(rsig, load_f64_m128d_s(s_hi));
    let res_hi = add_m128d_s(med, res_int);
    let k2 = add_m128d_s(sub_m128d_s(load_f64_m128d_s(s_hi), res_int), rsig);
    let k3 = add_m128d_s(sub_m128d_s(res_int, res_hi), med);
    let sum = add_m128d_s(add_m128d_s(add_m128d_s(corr, k2), k3), pol);
    let sum = add_m128d_s(sum, unpack_high_m128d(pol, pol));
    get_f64_from_m128d_s(add_m128d_s(res_hi, sum))
}

/// tan(x + lo) where x = (N + 16 quarters) pi/32 + r, N = x 32/pi rounded half away from zero. `lo` is the low part
/// of a reduced argument, or -0 (which adds nothing). As in `sin_kernel`, values are in a vector's low half unless
/// said otherwise.
#[inline(always)]
fn tan_kernel(x: f64, lo: f64, quarters: i32) -> f64 {
    let one = set_m128d_s(1.0);
    let xx = set_splat_m128d(x);
    let half = bitor_m128d(
        set_splat_m128d(0.5),
        bitand_m128d(set_splat_m128d(-0.0), xx),
    );
    // (x 2/pi, x 32/pi) rounded half away from zero: N2 and N.
    let y = add_m128d(
        mul_m128d(m128d::from_array([TWO_OVER_PI, THIRTY_TWO_OVER_PI]), xx),
        half,
    );
    let n = truncate_to_i32_m128d_s(unpack_high_m128d(y, y));
    // k = (16 N2, N), the quarter turn's part and N.
    let k = mul_m128d(
        convert_to_m128d_from_lower2_i32_m128i(truncate_m128d_to_m128i(y)),
        set_m128d(1.0, 16.0),
    );
    let row = &TAN.0[(n.wrapping_add(16 * quarters) & 31) as usize];
    let [.., t_hi, t_lo, sigma, t_hl, tau, mask] = row;
    let m1 = mul_m128d(m128d::from_array(TP1), k);
    let m2 = mul_m128d(m128d::from_array(TP2), k);
    let m3 = mul_m128d(k, m128d::from_array(TP3));
    let a = sub_m128d(xx, m1);
    // h: x's distance to the nearest multiple of pi/2, cut to 34 bits so that 1 - h (1/h) is nearly exact.
    let h = bitand_m128d(
        add_m128d_s(mul_m128d_s(set_m128d_s(QQ_2), k), a),
        m128d::from_bits([MASK_35; 2]),
    );
    let recip = div_m128d_s(one, h);
    // r = a - m2 in both halves and what it misses, c, with c0 = c + (r - h) in the low half.
    let r = sub_m128d(a, m2);
    let c = add_m128d(
        sub_m128d(sub_m128d(sub_m128d(a, r), m2), m3),
        set_splat_m128d(lo),
    );
    let c = add_m128d_s(c, sub_m128d_s(r, h));
    // The polynomial in b = r's high half, c8.. in the low half and c0.. in the high half, in the stub's order.
    let b = unpack_high_m128d(r, r);
    let b2 = mul_m128d(b, b);
    let b4 = mul_m128d(b2, b2);
    let b5 = mul_m128d(b, b4);
    let b8 = mul_m128d_s(b4, b4);
    let coef = |i: usize| pair(row, 2 * i);
    let p = add_m128d(
        add_m128d(mul_m128d(coef(1), b), coef(0)),
        mul_m128d(add_m128d(mul_m128d(coef(3), b), coef(2)), b2),
    );
    let q = add_m128d(
        add_m128d(mul_m128d(coef(6), b), coef(5)),
        mul_m128d(coef(7), b2),
    );
    let p = add_m128d(add_m128d(p, mul_m128d(coef(4), b4)), mul_m128d(q, b5));
    let pol = add_m128d_s(mul_m128d_s(b8, p), unpack_high_m128d(p, p));
    // b (sigma + t_hl) as a compensated sum.
    let med = mul_m128d(b, pair(row, 18));
    let med_t = unpack_high_m128d(med, med);
    let med_hi = add_m128d_s(med, med_t);
    let med_lo = add_m128d_s(med_t, sub_m128d_s(med, med_hi));
    let tail = mul_m128d_s(
        add_m128d_s(load_f64_m128d_s(sigma), load_f64_m128d_s(t_hl)),
        unpack_high_m128d(c, c),
    );
    let tail = add_m128d_s(add_m128d_s(tail, load_f64_m128d_s(t_lo)), med_lo);
    let pol = add_m128d_s(pol, tail);
    // tau / (h + c0), as recip_hi plus a correction.
    let recip_hi = bitand_m128d(load_f64_m128d_s(mask), recip);
    let err = mul_m128d_s(
        sub_m128d_s(
            sub_m128d_s(one, mul_m128d_s(h, recip_hi)),
            mul_m128d_s(c, recip),
        ),
        mul_m128d_s(recip, load_f64_m128d_s(tau)),
    );
    let t = sub_m128d_s(recip_hi, load_f64_m128d_s(t_hi));
    let s = sub_m128d_s(med_hi, t);
    let res = add_m128d_s(pol, sub_m128d_s(med_hi, add_m128d_s(t, s)));
    get_f64_from_m128d_s(add_m128d_s(sub_m128d_s(res, err), s))
}

/// x - k pi/4 (sin and cos, `turn_bits` 3) or x - k pi/2 (tan, `turn_bits` 2) for an argument beyond the main
/// path, as `hi + lo`, and k mod 2^turn_bits as a number between -7 and 7: the stub's reduction with 224 bits of 2/pi,
/// step by step. k is x 4/pi (or x 2/pi) rounded to the nearest integer, so |hi + lo| <= pi/8 (pi/4).
fn reduce(x: f64, turn_bits: u32, pi_4_hi: f64, pi_4_lo: f64) -> (f64, f64, i32) {
    let bits = x.to_bits();
    let sign = (bits >> 48) as u32 & 0x8000;
    let biased = (bits >> 52) as u32 & 0x7ff;
    // The significand a 2^32 + b and the seven words of 2/pi from word w (TWO_OVER_PI_BITS starts with two zero words).
    let a = u64::from(((bits >> 21) as u32 | 0x8000_0000) >> 11);
    let b = bits & 0xffff_ffff;
    let w = ((biased - 1014) >> 5) as usize;
    let word = |i: usize| u64::from(TWO_OVER_PI_BITS[w + i]);
    // Their product's bits 32 to 223, as six 32-bit limbs: word i times a and word i - 1 times b go into one limb.
    // The stub leaves out the lowest partial product (word 6 times b) and everything above bit 223.
    let mut limb = [0u32; 6];
    let mut carry = 0u64;
    for (k, limb) in limb.iter_mut().enumerate() {
        let low = word(5 - k) * b;
        let sum = carry + (low & 0xffff_ffff) + word(6 - k) * a;
        *limb = sum as u32;
        carry = (sum >> 32) + (low >> 32);
    }
    let mut mid = u64::from(limb[2]) | u64::from(limb[3]) << 32;
    let mut low = u64::from(limb[0]) | u64::from(limb[1]) << 32;
    // Where the product's binary point lies in limbs 5 and 4.
    let point = 32 * w as i32 + 19 - (biased as i32 - 1023);
    // The fraction is f 2^-exponent, f with 30 significant bits after the normalisation below.
    let mut exponent = point + 32;
    let fraction_bits = 32 - turn_bits;
    // k's bits (on top), and whether the fraction is at least a half: then k is rounded up and the fraction becomes
    // its complement, of the opposite sign.
    let mut turns;
    let mut high;
    let negated;
    if point >= turn_bits as i32 - 2 {
        let shift = (29 - point) as u32;
        turns = limb[5] << shift;
        let fraction = turns & ((1 << fraction_bits) - 1);
        high = u64::from(fraction >> shift) << 32 | u64::from(limb[4]);
        negated = fraction & 1 << (fraction_bits - 1) != 0;
        if negated {
            turns = turns.wrapping_add(1 << fraction_bits);
            (high, mid, low) = complement(
                u64::from((1u32 << fraction_bits) >> shift) << 32,
                high,
                mid,
                low,
            );
        }
    } else {
        let shift = (turn_bits as i32 - 3 - point) as u32;
        let shifted = (u64::from(limb[5]) << 32 | u64::from(limb[4])) << shift;
        turns = (shifted >> turn_bits) as u32;
        high = u64::from(shifted as u32 >> shift);
        negated = shifted & 0x8000_0000 != 0;
        if negated {
            turns = turns.wrapping_add(1 << fraction_bits);
            (high, mid, low) = complement(0x1_0000_0000 >> shift, high, mid, low);
        }
    }
    // Whole zero words out (the fraction of a double's x 4/pi never starts with 128 zero bits), then the leading
    // one to bit 29.
    for _ in 0..2 {
        if high == 0 {
            exponent += 64;
            (high, mid, low) = (mid, low, 0);
        }
    }
    if high != 0 {
        let shift = high.leading_zeros() as i32 - 34;
        if shift > 0 {
            high = high << shift | mid >> (64 - shift);
            mid = mid << shift | low >> (64 - shift);
        } else if shift < 0 {
            mid = mid >> -shift | high << (64 + shift);
            high >>= -shift;
        }
        exponent += shift;
    }
    // f 2^-exponent as two doubles, the sign of x (flipped for a complement) on both, times pi/4.
    let scale = ((16368 - 16 * exponent) as u32 | sign) ^ if negated { 0x8000 } else { 0 };
    let hi = high as i64 as f64 * f64::from_bits(u64::from(scale & 0xffff) << 48);
    let lo = (mid >> 1) as i64 as f64
        * f64::from_bits(u64::from(scale.wrapping_sub(1008) & 0xffff) << 48);
    let hi_pi = hi * pi_4_hi;
    let t = pi_4_lo * (hi + lo) + lo * pi_4_hi;
    let r_hi = hi_pi + t;
    let r_lo = t + (hi_pi - r_hi);
    let k = (turns >> fraction_bits) as i32;
    (r_hi, r_lo, if sign != 0 { -k } else { k })
}

/// `one` 2^128 - (high 2^128 + mid 2^64 + low): the stub's three-word subtraction.
fn complement(one: u64, high: u64, mid: u64, low: u64) -> (u64, u64, u64) {
    let below = u128::from(mid) << 64 | u128::from(low);
    let negated = below.wrapping_neg();
    let high = one.wrapping_sub(high).wrapping_sub(u64::from(below != 0));
    (high, (negated >> 64) as u64, negated as u64)
}

/// sin's and cos's table as the stub stores it: c_hl, s_hi, s_lo and sigma for each B = j pi/32, where
/// cos B = sigma + c_hl, with sigma the power of two nearest cos B, and sin B = s_hi + s_lo.
const SIN_COS_TABLE: [[u64; 4]; 64] = [
    [
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbf73_b92e_176d_6d31,
        0x3fb9_17a6_bc29_b42c,
        0xbc3e_2718_e000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbf93_ad06_0114_69fb,
        0x3fc8_f8b8_3c69_a60b,
        0xbc62_6d19_c000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbfa6_0bea_939d_225a,
        0x3fd2_9406_2ed5_9f06,
        0xbc75_d28d_a000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbfb3_7ca1_866b_95cf,
        0x3fd8_7de2_a6ae_a963,
        0xbc67_2ced_e000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbfbe_3a68_73fa_1279,
        0x3fde_2b5d_3806_f63b,
        0x3c5e_0d89_2000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbfc5_9267_5bc5_7974,
        0x3fe1_c73b_39ae_68c8,
        0x3c8b_25dd_2000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbfcd_0dfe_53ab_a2fd,
        0x3fe4_4cf3_2509_1dd6,
        0x3c68_076a_2000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0x3fca_8279_99fc_ef32,
        0x3fe6_a09e_667f_3bcd,
        0xbc8b_dd34_2000_0000,
        0x3fe0_0000_0000_0000,
    ],
    [
        0x3fc1_33cc_9424_7758,
        0x3fe8_bc80_6b15_1741,
        0xbc82_c5e1_2000_0000,
        0x3fe0_0000_0000_0000,
    ],
    [
        0x3fac_73b3_9ae6_8c87,
        0x3fea_9b66_290e_a1a3,
        0x3c39_f630_e000_0000,
        0x3fe0_0000_0000_0000,
    ],
    [
        0xbf9d_4a2c_7f90_9c4e,
        0x3fec_38b2_f180_bdb1,
        0xbc76_e0b1_8000_0000,
        0x3fe0_0000_0000_0000,
    ],
    [
        0xbfbe_0875_6545_5a75,
        0x3fed_906b_cf32_8d46,
        0x3c74_57e6_2000_0000,
        0x3fe0_0000_0000_0000,
    ],
    [
        0x3fa4_a031_76ac_f82d,
        0x3fee_9f41_56c6_2dda,
        0x3c87_60b1_e000_0000,
        0x3fd0_0000_0000_0000,
    ],
    [
        0xbfac_1d1f_0e59_67d5,
        0x3fef_6297_cff7_5cb0,
        0x3c75_6217_2000_0000,
        0x3fd0_0000_0000_0000,
    ],
    [
        0xbf9b_a165_0f59_2f50,
        0x3fef_d88d_a3d1_2526,
        0xbc88_7df6_4000_0000,
        0x3fc0_0000_0000_0000,
    ],
    [
        0x0000_0000_0000_0000,
        0x3ff0_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0x3f9b_a165_0f59_2f50,
        0x3fef_d88d_a3d1_2526,
        0xbc88_7df6_4000_0000,
        0xbfc0_0000_0000_0000,
    ],
    [
        0x3fac_1d1f_0e59_67d5,
        0x3fef_6297_cff7_5cb0,
        0x3c75_6217_2000_0000,
        0xbfd0_0000_0000_0000,
    ],
    [
        0xbfa4_a031_76ac_f82d,
        0x3fee_9f41_56c6_2dda,
        0x3c87_60b1_e000_0000,
        0xbfd0_0000_0000_0000,
    ],
    [
        0x3fbe_0875_6545_5a75,
        0x3fed_906b_cf32_8d46,
        0x3c74_57e6_2000_0000,
        0xbfe0_0000_0000_0000,
    ],
    [
        0x3f9d_4a2c_7f90_9c4e,
        0x3fec_38b2_f180_bdb1,
        0xbc76_e0b1_8000_0000,
        0xbfe0_0000_0000_0000,
    ],
    [
        0xbfac_73b3_9ae6_8c87,
        0x3fea_9b66_290e_a1a3,
        0x3c39_f630_e000_0000,
        0xbfe0_0000_0000_0000,
    ],
    [
        0xbfc1_33cc_9424_7758,
        0x3fe8_bc80_6b15_1741,
        0xbc82_c5e1_2000_0000,
        0xbfe0_0000_0000_0000,
    ],
    [
        0xbfca_8279_99fc_ef32,
        0x3fe6_a09e_667f_3bcd,
        0xbc8b_dd34_2000_0000,
        0xbfe0_0000_0000_0000,
    ],
    [
        0x3fcd_0dfe_53ab_a2fd,
        0x3fe4_4cf3_2509_1dd6,
        0x3c68_076a_2000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3fc5_9267_5bc5_7974,
        0x3fe1_c73b_39ae_68c8,
        0x3c8b_25dd_2000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3fbe_3a68_73fa_1279,
        0x3fde_2b5d_3806_f63b,
        0x3c5e_0d89_2000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3fb3_7ca1_866b_95cf,
        0x3fd8_7de2_a6ae_a963,
        0xbc67_2ced_e000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3fa6_0bea_939d_225a,
        0x3fd2_9406_2ed5_9f06,
        0xbc75_d28d_a000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3f93_ad06_0114_69fb,
        0x3fc8_f8b8_3c69_a60b,
        0xbc62_6d19_c000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3f73_b92e_176d_6d31,
        0x3fb9_17a6_bc29_b42c,
        0xbc3e_2718_e000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3f73_b92e_176d_6d31,
        0xbfb9_17a6_bc29_b42c,
        0x3c3e_2718_e000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3f93_ad06_0114_69fb,
        0xbfc8_f8b8_3c69_a60b,
        0x3c62_6d19_c000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3fa6_0bea_939d_225a,
        0xbfd2_9406_2ed5_9f06,
        0x3c75_d28d_a000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3fb3_7ca1_866b_95cf,
        0xbfd8_7de2_a6ae_a963,
        0x3c67_2ced_e000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3fbe_3a68_73fa_1279,
        0xbfde_2b5d_3806_f63b,
        0xbc5e_0d89_2000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3fc5_9267_5bc5_7974,
        0xbfe1_c73b_39ae_68c8,
        0xbc8b_25dd_2000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0x3fcd_0dfe_53ab_a2fd,
        0xbfe4_4cf3_2509_1dd6,
        0xbc68_076a_2000_0000,
        0xbff0_0000_0000_0000,
    ],
    [
        0xbfca_8279_99fc_ef32,
        0xbfe6_a09e_667f_3bcd,
        0x3c8b_dd34_2000_0000,
        0xbfe0_0000_0000_0000,
    ],
    [
        0xbfc1_33cc_9424_7758,
        0xbfe8_bc80_6b15_1741,
        0x3c82_c5e1_2000_0000,
        0xbfe0_0000_0000_0000,
    ],
    [
        0xbfac_73b3_9ae6_8c87,
        0xbfea_9b66_290e_a1a3,
        0xbc39_f630_e000_0000,
        0xbfe0_0000_0000_0000,
    ],
    [
        0x3f9d_4a2c_7f90_9c4e,
        0xbfec_38b2_f180_bdb1,
        0x3c76_e0b1_8000_0000,
        0xbfe0_0000_0000_0000,
    ],
    [
        0x3fbe_0875_6545_5a75,
        0xbfed_906b_cf32_8d46,
        0xbc74_57e6_2000_0000,
        0xbfe0_0000_0000_0000,
    ],
    [
        0xbfa4_a031_76ac_f82d,
        0xbfee_9f41_56c6_2dda,
        0xbc87_60b1_e000_0000,
        0xbfd0_0000_0000_0000,
    ],
    [
        0x3fac_1d1f_0e59_67d5,
        0xbfef_6297_cff7_5cb0,
        0xbc75_6217_2000_0000,
        0xbfd0_0000_0000_0000,
    ],
    [
        0x3f9b_a165_0f59_2f50,
        0xbfef_d88d_a3d1_2526,
        0x3c88_7df6_4000_0000,
        0xbfc0_0000_0000_0000,
    ],
    [
        0x0000_0000_0000_0000,
        0xbff0_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0xbf9b_a165_0f59_2f50,
        0xbfef_d88d_a3d1_2526,
        0x3c88_7df6_4000_0000,
        0x3fc0_0000_0000_0000,
    ],
    [
        0xbfac_1d1f_0e59_67d5,
        0xbfef_6297_cff7_5cb0,
        0xbc75_6217_2000_0000,
        0x3fd0_0000_0000_0000,
    ],
    [
        0x3fa4_a031_76ac_f82d,
        0xbfee_9f41_56c6_2dda,
        0xbc87_60b1_e000_0000,
        0x3fd0_0000_0000_0000,
    ],
    [
        0xbfbe_0875_6545_5a75,
        0xbfed_906b_cf32_8d46,
        0xbc74_57e6_2000_0000,
        0x3fe0_0000_0000_0000,
    ],
    [
        0xbf9d_4a2c_7f90_9c4e,
        0xbfec_38b2_f180_bdb1,
        0x3c76_e0b1_8000_0000,
        0x3fe0_0000_0000_0000,
    ],
    [
        0x3fac_73b3_9ae6_8c87,
        0xbfea_9b66_290e_a1a3,
        0xbc39_f630_e000_0000,
        0x3fe0_0000_0000_0000,
    ],
    [
        0x3fc1_33cc_9424_7758,
        0xbfe8_bc80_6b15_1741,
        0x3c82_c5e1_2000_0000,
        0x3fe0_0000_0000_0000,
    ],
    [
        0x3fca_8279_99fc_ef32,
        0xbfe6_a09e_667f_3bcd,
        0x3c8b_dd34_2000_0000,
        0x3fe0_0000_0000_0000,
    ],
    [
        0xbfcd_0dfe_53ab_a2fd,
        0xbfe4_4cf3_2509_1dd6,
        0xbc68_076a_2000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbfc5_9267_5bc5_7974,
        0xbfe1_c73b_39ae_68c8,
        0xbc8b_25dd_2000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbfbe_3a68_73fa_1279,
        0xbfde_2b5d_3806_f63b,
        0xbc5e_0d89_2000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbfb3_7ca1_866b_95cf,
        0xbfd8_7de2_a6ae_a963,
        0x3c67_2ced_e000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbfa6_0bea_939d_225a,
        0xbfd2_9406_2ed5_9f06,
        0x3c75_d28d_a000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbf93_ad06_0114_69fb,
        0xbfc8_f8b8_3c69_a60b,
        0x3c62_6d19_c000_0000,
        0x3ff0_0000_0000_0000,
    ],
    [
        0xbf73_b92e_176d_6d31,
        0xbfb9_17a6_bc29_b42c,
        0x3c3e_2718_e000_0000,
        0x3ff0_0000_0000_0000,
    ],
];
/// 2/pi's bits after the binary point, 32 to a word, after two zero words.
const TWO_OVER_PI_BITS: [u32; 41] = [
    0x0000_0000,
    0x0000_0000,
    0xa2f9_836e,
    0x4e44_1529,
    0xfc27_57d1,
    0xf534_ddc0,
    0xdb62_9599,
    0x3c43_9041,
    0xfe51_63ab,
    0xdebb_c561,
    0xb724_6e3a,
    0x424d_d2e0,
    0x0649_2eea,
    0x09d1_921c,
    0xfe1d_eb1c,
    0xb129_a73e,
    0xe882_35f5,
    0x2ebb_4484,
    0xe99c_7026,
    0xb45f_7e41,
    0x3991_d639,
    0x8353_39f4,
    0x9c84_5f8b,
    0xbdf9_283b,
    0x1ff8_97ff,
    0xde05_980f,
    0xef2f_118b,
    0x5a0a_6d1f,
    0x6d36_7ecf,
    0x27cb_09b7,
    0x4f46_3f66,
    0x9e5f_ea2d,
    0x7527_bac7,
    0xebe5_f17b,
    0x3d07_39f7,
    0x8a52_92ea,
    0x6bfb_5fb1,
    0x1f8d_5d08,
    0x5603_3046,
    0xfc7b_6bab,
    0xf0cf_bc21,
];
/// tan's table as the stub stores it, for each B = j pi/32: the polynomial c0 + c1 r + ... + c15 r^15 in pairs
/// (c8, c0), (c9, c1) ... (c15, c7), c0 and c1 zero; the constant term t_hi + t_lo; the coefficient of r,
/// sigma + t_hl; and the reciprocal's factor tau (1 or 0) with the mask that splits it.
const TAN_TABLE: [[u64; 22]; 32] = [
    [
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x3f96_64f4_882c_10fa,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x3f82_26e3_55e6_c23d,
        0x3fd5_5555_5555_5555,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x3f6d_6d3d_0e15_7de0,
        0x3fc1_1111_1111_1111,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x3f57_da36_452b_75e3,
        0x3fab_a1ba_1ba1_ba1c,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x3ff0_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0x3f95_3f83_4e43_5f9b,
        0x0000_0000_0000_0000,
        0x3f9b_74ea_3c6e_8e46,
        0x0000_0000_0000_0000,
        0x3f85_ad63_da5b_7511,
        0x3fb9_7558_dc23_0b9b,
        0x3f88_1308_26cb_3788,
        0x3fd6_2ac9_76fc_4985,
        0x3f75_7c85_77bb_08ba,
        0x3fb1_381e_b624_7521,
        0x3f75_4e95_5922_170c,
        0x3fc2_7f83_8746_482d,
        0x3f64_e391_1105_5b30,
        0x3fa3_e609_3e66_6320,
        0x3f63_01df_0de9_dae3,
        0x3faf_a8ae_1f1d_ca06,
        0x3fb9_36bb_8c5b_2da2,
        0x3c58_7d05_4e88_f7a5,
        0x3ff0_0000_0000_0000,
        0x3f83_dde2_a893_5dd9,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0x3faa_3407_5a27_9ea3,
        0x0000_0000_0000_0000,
        0x3fa7_0153_432d_65fa,
        0x0000_0000_0000_0000,
        0x3f9d_03ef_891a_4602,
        0x3fca_77d9_d62c_a5f8,
        0x3f97_a265_b35f_4628,
        0x3fd8_cf51_4332_58fa,
        0x3f8f_88e3_b58f_d909,
        0x3fc2_b154_0177_1cea,
        0x3f88_8f57_f356_2f8e,
        0x3fc7_370f_c028_a723,
        0x3f80_f44c_20b7_f9f0,
        0x3fb6_dfaa_2143_68e9,
        0x3f79_b4b6_2889_1863,
        0x3fb6_cb8e_172d_bbf0,
        0x3fc9_75f5_e055_3158,
        0x3c2e_f5d3_593f_e814,
        0x3ff0_0000_0000_0000,
        0x3fa4_4203_03de_c550,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0x3fbb_8ec5_9314_533e,
        0x0000_0000_0000_0000,
        0x3fb6_d3f4_09aa_36d0,
        0x0000_0000_0000_0000,
        0x3fb1_3950_dcb4_27fd,
        0x3fd5_335e_d87a_b0bb,
        0x3fab_b382_ce0a_e8a5,
        0x3fdd_ba41_7914_3126,
        0x3fa5_52f1_5f2b_28d4,
        0x3fd0_15ab_59f2_1a6d,
        0x3fa0_e984_22c2_7d95,
        0x3fd0_576c_e19f_c6aa,
        0x3f9a_4898_8f2c_2950,
        0x3fc5_9462_c0b3_f22c,
        0x3f94_b61c_1883_a4b8,
        0x3fc3_0eb8_3f83_8640,
        0x3fd3_6a08_355c_63dc,
        0xbc6d_704d_1dce_993d,
        0x3ff0_0000_0000_0000,
        0x3fb7_8e92_2b82_ab63,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0x3fcc_fc56_56f3_7042,
        0x0000_0000_0000_0000,
        0x3fc9_0125_aa56_3951,
        0x0000_0000_0000_0000,
        0x3fc5_0533_3d0e_7c5d,
        0x3fdf_0ed9_9bed_9b2e,
        0x3fc1_f250_5fe7_c47c,
        0x3fe2_edd9_96c1_25e5,
        0x3fbe_5c71_5a02_bbd8,
        0x3fda_08b7_8636_2c20,
        0x3fb9_d342_4b44_35ed,
        0x3fd9_11bd_4b49_4091,
        0x3fb5_e4c7_b566_58be,
        0x3fd3_c092_93a2_fd76,
        0x3fb2_9910_da27_1794,
        0x3fd1_89be_3303_df2b,
        0x3fda_8279_99fc_ef32,
        0x3c70_8b2f_b68c_1467,
        0x3ff0_0000_0000_0000,
        0x3fc5_f619_980c_4337,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0x3fdf_f10f_cc03_e501,
        0x0000_0000_0000_0000,
        0x3fdd_b63b_44a4_e845,
        0x0000_0000_0000_0000,
        0x3fdb_72a4_3768_ad9f,
        0x3fe5_fdb9_3dd0_1cca,
        0x3fd9_72b2_a61d_2811,
        0x3fe9_77f9_5645_ad0b,
        0x3fd7_8ca3_d013_b3ab,
        0x3fe4_f192_bf0b_f914,
        0x3fd5_d060_4d53_e730,
        0x3fe4_9933_3f8b_9000,
        0x3fd4_322a_e2b8_2f08,
        0x3fe2_7ae1_5936_a835,
        0x3fd2_b3fb_b1c6_1c9b,
        0x3fe1_659e_ef47_8605,
        0x3fe1_1ab7_1908_34ec,
        0xbc8e_564b_cdb6_25ea,
        0x3ff0_0000_0000_0000,
        0x3fd2_48f1_b072_17e3,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0x3ff2_de9c_2b2c_49d0,
        0x0000_0000_0000_0000,
        0x3ff3_3e58_2655_bc98,
        0x0000_0000_0000_0000,
        0x3ff3_972e_ff69_1fa2,
        0x3fee_ed87_e934_63bd,
        0x3ff3_f5b2_070e_10a0,
        0x3ff2_0c10_f4d7_90a4,
        0x3ff4_541a_a04e_8ea3,
        0x3ff1_369e_386a_ccd3,
        0x3ff4_b521_222a_66dd,
        0x3ff2_0817_22a9_777e,
        0x3ff5_178f_52a0_4a6e,
        0x3ff2_2137_ddaa_0031,
        0x3ff5_7c01_4447_d47c,
        0x3ff2_9311_1e9c_7f1d,
        0x3fe5_61b8_2ab7_f990,
        0x3c87_a8c5_209c_7df1,
        0x3ff0_0000_0000_0000,
        0x3fdc_92d8_4170_bcc6,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0x4008_5e24_c7ab_4d5a,
        0x0000_0000_0000_0000,
        0x400b_963d_e93e_a75d,
        0x0000_0000_0000_0000,
        0x400f_37e2_94a7_f25a,
        0x3ff5_f984_4b62_61cb,
        0x4011_aab0_5a9d_d812,
        0x3ffa_f5a5_74c3_0018,
        0x4013_fe8b_7f2c_e8e3,
        0x3ffd_7334_fe8e_54fa,
        0x4016_a10c_670d_618d,
        0x4000_e012_4db9_7058,
        0x4019_9c5f_24df_44dd,
        0x4003_006e_697d_6ece,
        0x401c_fc4d_8329_8b82,
        0x4005_8c19_19d4_90d6,
        0x3fea_4300_2ae4_2850,
        0xbc7a_6db8_118e_20e6,
        0x4000_0000_0000_0000,
        0xbfd4_e526_e333_45b8,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0x4021_9659_6596_5966,
        0x0000_0000_0000_0000,
        0x4026_64f4_882c_10fa,
        0x0000_0000_0000_0000,
        0x402c_8342_83cd_3723,
        0x4000_0000_0000_0000,
        0x4032_26e3_55e6_c23d,
        0x4005_5555_5555_5555,
        0x4037_1c96_3445_1939,
        0x400a_aaaa_aaaa_aaab,
        0x403d_6d3d_0e15_7de0,
        0x4011_1111_1111_1111,
        0x4042_bbce_a738_201f,
        0x4015_b05b_05b0_5b06,
        0x4047_da36_452b_75e3,
        0x401b_a1ba_1ba1_ba1c,
        0x3ff0_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x4000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0xbf33_eaf9_4f48_b8d3,
        0x0000_0000_0000_0000,
        0x3f20_b8ea_0cf7_586f,
        0x0000_0000_0000_0000,
        0xbf0a_baf3_d025_8911,
        0xbfab_5a8c_23e4_9fe9,
        0x3ef6_0d15_2d53_222e,
        0x3fa1_72b2_2116_9451,
        0xbee1_d3b5_bb25_4dbc,
        0xbf84_c7db_dbf9_3b8e,
        0x3ecd_3364_05b4_630b,
        0x3f74_3924_ee9a_ada7,
        0xbeb7_b7b9_794a_8297,
        0xbf5d_41f5_e015_f797,
        0x3ea3_5dfb_e41a_4a56,
        0x3f49_a2ab_e4c2_a251,
        0xbfce_49ce_5af9_e000,
        0x3d1e_b860_8c74_3719,
        0x0000_0000_0000_0000,
        0x3fd7_8294_1b48_63cf,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0xbf2b_9320_535a_d890,
        0x0000_0000_0000_0000,
        0x3f16_d61d_018f_df1f,
        0x0000_0000_0000_0000,
        0xbf01_39e4_0359_f1be,
        0xbfa6_7e17_a431_7c6d,
        0x3eeb_b405_8267_2d0f,
        0x3f9f_455b_2f1b_621e,
        0xbed5_5317_51cc_f238,
        0xbf80_4bee_f437_b9ac,
        0x3ec0_e993_c791_a2b5,
        0x3f70_80c2_919a_1db2,
        0xbeaa_48a2_336a_5b0e,
        0xbf55_a443_0a26_8358,
        0x3e94_b61f_dfd9_78e4,
        0x3f43_1806_d776_7a58,
        0xbfc9_bbe8_2aea_0000,
        0xbd3a_2369_7723_ea61,
        0x0000_0000_0000_0000,
        0x3fd6_e642_df77_96ff,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0xbf23_1c78_b9ff_07ce,
        0x0000_0000_0000_0000,
        0x3f0f_f0e0_a551_7182,
        0x0000_0000_0000_0000,
        0xbef6_6191_790b_4cbc,
        0xbfa2_1ac0_848a_46c6,
        0x3ee1_d3ec_b164_35fa,
        0x3f9c_71ea_2a1a_a832,
        0xbec9_dd1a_fdd2_99ef,
        0xbf79_3363_3f8d_baaf,
        0x3eb4_15d6_309f_c6ea,
        0x3f6b_83ba_bee6_0471,
        0xbe9d_ae11_94a0_a697,
        0xbf4f_d07b_3e5c_67b3,
        0x3e86_bd75_9a8f_3e3e,
        0x3f3d_1eb1_a4be_b7a4,
        0xbfc5_49ce_29cf_c000,
        0xbd39_7b33_bf15_9358,
        0x0000_0000_0000_0000,
        0x3fd6_66f0_871f_ee6c,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0xbf1a_3958_7d98_a556,
        0x0000_0000_0000_0000,
        0x3f07_04c2_9d88_dc01,
        0x0000_0000_0000_0000,
        0xbeed_054a_7374_2a2b,
        0xbf9c_2a13_5884_4587,
        0x3ed7_a326_5568_8a79,
        0x3f9a_48f4_ee33_f1d6,
        0xbebf_8939_a8dc_9888,
        0xbf72_f746_aad4_b5b8,
        0x3ea8_8f82_9102_efa1,
        0x3f67_8228_dabc_29cf,
        0xbe90_f456_9289_afb8,
        0xbf46_f3a3_741f_b4ed,
        0x3e79_b4bf_a97f_6663,
        0x3f36_db70_ca89_ff3f,
        0xbfc0_ee13_a8a2_a000,
        0xbd33_8b9f_3da2_4be1,
        0x0000_0000_0000_0000,
        0x3fd6_01fd_11cd_6c69,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0xbf11_6b01_1a15_4b97,
        0x0000_0000_0000_0000,
        0x3f01_47bf_2d42_7630,
        0x0000_0000_0000_0000,
        0xbee2_64d4_b938_20c8,
        0xbf94_ab8c_bb6c_bb18,
        0x3ed0_568b_888d_4d92,
        0x3f98_b19b_6073_0f7c,
        0xbeb2_f950_e4b1_fb11,
        0xbf6b_21cd_22cf_9f74,
        0x3e9f_499e_4a3f_f0a6,
        0x3f64_aad7_fd2b_83ce,
        0xbe83_487c_637b_73af,
        0xbf3f_c092_e522_591a,
        0x3e6e_3aae_a158_e8bc,
        0x3f32_9d2f_e5e8_2ffa,
        0xbfb9_477f_d636_a000,
        0xbd13_5ef9_c2c2_d2bc,
        0x0000_0000_0000_0000,
        0x3fd5_b566_f2fd_b123,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0xbf05_448d_c41a_cb64,
        0x0000_0000_0000_0000,
        0x3efb_7ad2_dbb0_3d6f,
        0x0000_0000_0000_0000,
        0xbed5_aea5_9e42_962d,
        0xbf8b_2398_2579_f8ef,
        0x3ec8_1441_288a_1ed9,
        0x3f97_9a3a_b019_8dc5,
        0xbea5_7cd3_2fdf_e253,
        0xbf61_7caa_5766_336f,
        0x3e95_4ed6_6009_44c3,
        0x3f62_c646_a4e0_aaf8,
        0xbe74_e3a3_6b8f_b29c,
        0xbf33_f952_dc4c_0409,
        0x3e63_01ec_9bff_e365,
        0x3f2f_c566_b886_9e44,
        0xbfb0_cc62_e1e0_4000,
        0xbd11_9cbc_016b_907f,
        0x0000_0000_0000_0000,
        0x3fd5_7fb3_e6b9_d8fa,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0xbef4_29d7_5daf_22a6,
        0x0000_0000_0000_0000,
        0x3ef7_a27d_06bc_a545,
        0x0000_0000_0000_0000,
        0xbec4_1c3e_7211_c19a,
        0xbf7a_e3f4_956e_d53e,
        0x3ec3_901b_ee75_0e72,
        0x3f96_f713_91d4_43f5,
        0xbe93_6e09_3666_1e6c,
        0xbf51_22e8_506f_9381,
        0x3e90_41b9_cb6d_d43f,
        0x3f61_b0c7_6698_b2ff,
        0xbe62_5a8a_576b_f12b,
        0xbf23_499d_e5a0_e9dc,
        0x3e5b_1c2c_1103_84dd,
        0x3f2c_b899_68d4_3db6,
        0xbfa0_c414_6eca_c000,
        0x3d13_500f_cd7d_d58c,
        0x0000_0000_0000_0000,
        0x3fd5_5fe0_85a2_c8fb,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x3ef6_6a8f_2bf7_0ebe,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x3ec2_2805_d644_267f,
        0x3f96_c16c_16c1_6c17,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x3e8d_6db2_c4e0_9162,
        0x3f61_566a_bc01_1567,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x3e57_da4e_1f79_955c,
        0x3f2b_bd77_9334_ef0b,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x3fd5_5555_5555_5555,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0x3ef4_29d7_5daf_22a6,
        0x0000_0000_0000_0000,
        0x3ef7_a27d_06bc_a545,
        0x0000_0000_0000_0000,
        0x3ec4_1c3e_7211_c19a,
        0x3f7a_e3f4_956e_d53e,
        0x3ec3_901b_ee75_0e72,
        0x3f96_f713_91d4_43f5,
        0x3e93_6e09_3666_1e6c,
        0x3f51_22e8_506f_9381,
        0x3e90_41b9_cb6d_d43f,
        0x3f61_b0c7_6698_b2ff,
        0x3e62_5a8a_576b_f12b,
        0x3f23_499d_e5a0_e9dc,
        0x3e5b_1c2c_1103_84dd,
        0x3f2c_b899_68d4_3db6,
        0x3fa0_c414_6eca_c000,
        0xbd13_500f_cd7d_d58c,
        0x0000_0000_0000_0000,
        0x3fd5_5fe0_85a2_c8fb,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0x3f05_448d_c41a_cb64,
        0x0000_0000_0000_0000,
        0x3efb_7ad2_dbb0_3d6f,
        0x0000_0000_0000_0000,
        0x3ed5_aea5_9e42_962d,
        0x3f8b_2398_2579_f8ef,
        0x3ec8_1441_288a_1ed9,
        0x3f97_9a3a_b019_8dc5,
        0x3ea5_7cd3_2fdf_e253,
        0x3f61_7caa_5766_336f,
        0x3e95_4ed6_6009_44c3,
        0x3f62_c646_a4e0_aaf8,
        0x3e74_e3a3_6b8f_b29c,
        0x3f33_f952_dc4c_0409,
        0x3e63_01ec_9bff_e365,
        0x3f2f_c566_b886_9e44,
        0x3fb0_cc62_e1e0_4000,
        0x3d11_9cbc_016b_907f,
        0x0000_0000_0000_0000,
        0x3fd5_7fb3_e6b9_d8fa,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0x3f11_6b01_1a15_4b97,
        0x0000_0000_0000_0000,
        0x3f01_47bf_2d42_7630,
        0x0000_0000_0000_0000,
        0x3ee2_64d4_b938_20c8,
        0x3f94_ab8c_bb6c_bb18,
        0x3ed0_568b_888d_4d92,
        0x3f98_b19b_6073_0f7c,
        0x3eb2_f950_e4b1_fb11,
        0x3f6b_21cd_22cf_9f74,
        0x3e9f_499e_4a3f_f0a6,
        0x3f64_aad7_fd2b_83ce,
        0x3e83_487c_637b_73af,
        0x3f3f_c092_e522_591a,
        0x3e6e_3aae_a158_e8bc,
        0x3f32_9d2f_e5e8_2ffa,
        0x3fb9_477f_d636_a000,
        0x3d13_5ef9_c2c2_d2bc,
        0x0000_0000_0000_0000,
        0x3fd5_b566_f2fd_b123,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0x3f1a_3958_7d98_a556,
        0x0000_0000_0000_0000,
        0x3f07_04c2_9d88_dc01,
        0x0000_0000_0000_0000,
        0x3eed_054a_7374_2a2b,
        0x3f9c_2a13_5884_4587,
        0x3ed7_a326_5568_8a79,
        0x3f9a_48f4_ee33_f1d6,
        0x3ebf_8939_a8dc_9888,
        0x3f72_f746_aad4_b5b8,
        0x3ea8_8f82_9102_efa1,
        0x3f67_8228_dabc_29cf,
        0x3e90_f456_9289_afb8,
        0x3f46_f3a3_741f_b4ed,
        0x3e79_b4bf_a97f_6663,
        0x3f36_db70_ca89_ff3f,
        0x3fc0_ee13_a8a2_a000,
        0x3d33_8b9f_3da2_4be1,
        0x0000_0000_0000_0000,
        0x3fd6_01fd_11cd_6c69,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0x3f23_1c78_b9ff_07ce,
        0x0000_0000_0000_0000,
        0x3f0f_f0e0_a551_7182,
        0x0000_0000_0000_0000,
        0x3ef6_6191_790b_4cbc,
        0x3fa2_1ac0_848a_46c6,
        0x3ee1_d3ec_b164_35fa,
        0x3f9c_71ea_2a1a_a832,
        0x3ec9_dd1a_fdd2_99ef,
        0x3f79_3363_3f8d_baaf,
        0x3eb4_15d6_309f_c6ea,
        0x3f6b_83ba_bee6_0471,
        0x3e9d_ae11_94a0_a697,
        0x3f4f_d07b_3e5c_67b3,
        0x3e86_bd75_9a8f_3e3e,
        0x3f3d_1eb1_a4be_b7a4,
        0x3fc5_49ce_29cf_c000,
        0x3d39_7b33_bf15_9358,
        0x0000_0000_0000_0000,
        0x3fd6_66f0_871f_ee6c,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0x3f2b_9320_535a_d890,
        0x0000_0000_0000_0000,
        0x3f16_d61d_018f_df1f,
        0x0000_0000_0000_0000,
        0x3f01_39e4_0359_f1be,
        0x3fa6_7e17_a431_7c6d,
        0x3eeb_b405_8267_2d0f,
        0x3f9f_455b_2f1b_621e,
        0x3ed5_5317_51cc_f238,
        0x3f80_4bee_f437_b9ac,
        0x3ec0_e993_c791_a2b5,
        0x3f70_80c2_919a_1db2,
        0x3eaa_48a2_336a_5b0e,
        0x3f55_a443_0a26_8358,
        0x3e94_b61f_dfd9_78e4,
        0x3f43_1806_d776_7a58,
        0x3fc9_bbe8_2aea_0000,
        0x3d3a_2369_7723_ea61,
        0x0000_0000_0000_0000,
        0x3fd6_e642_df77_96ff,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0x3f33_eaf9_4f48_b8d3,
        0x0000_0000_0000_0000,
        0x3f20_b8ea_0cf7_586f,
        0x0000_0000_0000_0000,
        0x3f0a_baf3_d025_8911,
        0x3fab_5a8c_23e4_9fe9,
        0x3ef6_0d15_2d53_222e,
        0x3fa1_72b2_2116_9451,
        0x3ee1_d3b5_bb25_4dbc,
        0x3f84_c7db_dbf9_3b8e,
        0x3ecd_3364_05b4_630b,
        0x3f74_3924_ee9a_ada7,
        0x3eb7_b7b9_794a_8297,
        0x3f5d_41f5_e015_f797,
        0x3ea3_5dfb_e41a_4a56,
        0x3f49_a2ab_e4c2_a251,
        0x3fce_49ce_5af9_e000,
        0xbd1e_b860_8c74_3719,
        0x0000_0000_0000_0000,
        0x3fd7_8294_1b48_63cf,
        0x3ff0_0000_0000_0000,
        0xffff_fff8_0000_0000,
    ],
    [
        0xc021_9659_6596_5966,
        0x0000_0000_0000_0000,
        0x4026_64f4_882c_10fa,
        0x0000_0000_0000_0000,
        0xc02c_8342_83cd_3723,
        0xc000_0000_0000_0000,
        0x4032_26e3_55e6_c23d,
        0x4005_5555_5555_5555,
        0xc037_1c96_3445_1939,
        0xc00a_aaaa_aaaa_aaab,
        0x403d_6d3d_0e15_7de0,
        0x4011_1111_1111_1111,
        0xc042_bbce_a738_201f,
        0xc015_b05b_05b0_5b06,
        0x4047_da36_452b_75e3,
        0x401b_a1ba_1ba1_ba1c,
        0xbff0_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x4000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0xc008_5e24_c7ab_4d5a,
        0x0000_0000_0000_0000,
        0x400b_963d_e93e_a75d,
        0x0000_0000_0000_0000,
        0xc00f_37e2_94a7_f25a,
        0xbff5_f984_4b62_61cb,
        0x4011_aab0_5a9d_d812,
        0x3ffa_f5a5_74c3_0018,
        0xc013_fe8b_7f2c_e8e3,
        0xbffd_7334_fe8e_54fa,
        0x4016_a10c_670d_618d,
        0x4000_e012_4db9_7058,
        0xc019_9c5f_24df_44dd,
        0xc003_006e_697d_6ece,
        0x401c_fc4d_8329_8b82,
        0x4005_8c19_19d4_90d6,
        0xbfea_4300_2ae4_2850,
        0x3c7a_6db8_118e_20e6,
        0x4000_0000_0000_0000,
        0xbfd4_e526_e333_45b8,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0xbff2_de9c_2b2c_49d0,
        0x0000_0000_0000_0000,
        0x3ff3_3e58_2655_bc98,
        0x0000_0000_0000_0000,
        0xbff3_972e_ff69_1fa2,
        0xbfee_ed87_e934_63bd,
        0x3ff3_f5b2_070e_10a0,
        0x3ff2_0c10_f4d7_90a4,
        0xbff4_541a_a04e_8ea3,
        0xbff1_369e_386a_ccd3,
        0x3ff4_b521_222a_66dd,
        0x3ff2_0817_22a9_777e,
        0xbff5_178f_52a0_4a6e,
        0xbff2_2137_ddaa_0031,
        0x3ff5_7c01_4447_d47c,
        0x3ff2_9311_1e9c_7f1d,
        0xbfe5_61b8_2ab7_f990,
        0xbc87_a8c5_209c_7df1,
        0x3ff0_0000_0000_0000,
        0x3fdc_92d8_4170_bcc6,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0xbfdf_f10f_cc03_e501,
        0x0000_0000_0000_0000,
        0x3fdd_b63b_44a4_e845,
        0x0000_0000_0000_0000,
        0xbfdb_72a4_3768_ad9f,
        0xbfe5_fdb9_3dd0_1cca,
        0x3fd9_72b2_a61d_2811,
        0x3fe9_77f9_5645_ad0b,
        0xbfd7_8ca3_d013_b3ab,
        0xbfe4_f192_bf0b_f914,
        0x3fd5_d060_4d53_e730,
        0x3fe4_9933_3f8b_9000,
        0xbfd4_322a_e2b8_2f08,
        0xbfe2_7ae1_5936_a835,
        0x3fd2_b3fb_b1c6_1c9b,
        0x3fe1_659e_ef47_8605,
        0xbfe1_1ab7_1908_34ec,
        0x3c8e_564b_cdb6_25ea,
        0x3ff0_0000_0000_0000,
        0x3fd2_48f1_b072_17e3,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0xbfcc_fc56_56f3_7042,
        0x0000_0000_0000_0000,
        0x3fc9_0125_aa56_3951,
        0x0000_0000_0000_0000,
        0xbfc5_0533_3d0e_7c5d,
        0xbfdf_0ed9_9bed_9b2e,
        0x3fc1_f250_5fe7_c47c,
        0x3fe2_edd9_96c1_25e5,
        0xbfbe_5c71_5a02_bbd8,
        0xbfda_08b7_8636_2c20,
        0x3fb9_d342_4b44_35ed,
        0x3fd9_11bd_4b49_4091,
        0xbfb5_e4c7_b566_58be,
        0xbfd3_c092_93a2_fd76,
        0x3fb2_9910_da27_1794,
        0x3fd1_89be_3303_df2b,
        0xbfda_8279_99fc_ef32,
        0xbc70_8b2f_b68c_1467,
        0x3ff0_0000_0000_0000,
        0x3fc5_f619_980c_4337,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0xbfbb_8ec5_9314_533e,
        0x0000_0000_0000_0000,
        0x3fb6_d3f4_09aa_36d0,
        0x0000_0000_0000_0000,
        0xbfb1_3950_dcb4_27fd,
        0xbfd5_335e_d87a_b0bb,
        0x3fab_b382_ce0a_e8a5,
        0x3fdd_ba41_7914_3126,
        0xbfa5_52f1_5f2b_28d4,
        0xbfd0_15ab_59f2_1a6d,
        0x3fa0_e984_22c2_7d95,
        0x3fd0_576c_e19f_c6aa,
        0xbf9a_4898_8f2c_2950,
        0xbfc5_9462_c0b3_f22c,
        0x3f94_b61c_1883_a4b8,
        0x3fc3_0eb8_3f83_8640,
        0xbfd3_6a08_355c_63dc,
        0x3c6d_704d_1dce_993d,
        0x3ff0_0000_0000_0000,
        0x3fb7_8e92_2b82_ab63,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0xbfaa_3407_5a27_9ea3,
        0x0000_0000_0000_0000,
        0x3fa7_0153_432d_65fa,
        0x0000_0000_0000_0000,
        0xbf9d_03ef_891a_4602,
        0xbfca_77d9_d62c_a5f8,
        0x3f97_a265_b35f_4628,
        0x3fd8_cf51_4332_58fa,
        0xbf8f_88e3_b58f_d909,
        0xbfc2_b154_0177_1cea,
        0x3f88_8f57_f356_2f8e,
        0x3fc7_370f_c028_a723,
        0xbf80_f44c_20b7_f9f0,
        0xbfb6_dfaa_2143_68e9,
        0x3f79_b4b6_2889_1863,
        0x3fb6_cb8e_172d_bbf0,
        0xbfc9_75f5_e055_3158,
        0xbc2e_f5d3_593f_e814,
        0x3ff0_0000_0000_0000,
        0x3fa4_4203_03de_c550,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
    [
        0xbf95_3f83_4e43_5f9b,
        0x0000_0000_0000_0000,
        0x3f9b_74ea_3c6e_8e46,
        0x0000_0000_0000_0000,
        0xbf85_ad63_da5b_7511,
        0xbfb9_7558_dc23_0b9b,
        0x3f88_1308_26cb_3788,
        0x3fd6_2ac9_76fc_4985,
        0xbf75_7c85_77bb_08ba,
        0xbfb1_381e_b624_7521,
        0x3f75_4e95_5922_170c,
        0x3fc2_7f83_8746_482d,
        0xbf64_e391_1105_5b30,
        0xbfa3_e609_3e66_6320,
        0x3f63_01df_0de9_dae3,
        0x3faf_a8ae_1f1d_ca06,
        0xbfb9_36bb_8c5b_2da2,
        0xbc58_7d05_4e88_f7a5,
        0x3ff0_0000_0000_0000,
        0x3f83_dde2_a893_5dd9,
        0x0000_0000_0000_0000,
        0x0000_0000_0000_0000,
    ],
];

#[cfg(test)]
mod tests {
    /// Checks `f` against (argument bits, result bits) pairs.
    fn check(name: &str, f: fn(f64) -> f64, cases: &[(u64, u64)]) {
        for &(x, expected) in cases {
            assert_eq!(f(f64::from_bits(x)).to_bits(), expected, "{name}({x:#x})");
        }
    }

    /// `Math.sin` on HotSpot 25.0.4.1 (x86-64): input and result bits. The first four are arguments in the usual
    /// range where glibc's `sin` gives the other neighbour; then the reduction of huge arguments, both edges of the
    /// main path, the smallest normal numbers (left as they are) and a negative zero.
    #[test]
    fn sin_is_hotspots() {
        check(
            "sin",
            super::sin,
            &[
                (0x4023_2b4d_079e_f530, 0xbfc4_5de0_d71a_5178),
                (0xc035_148c_0fcb_2483, 0xbfe9_4811_9a12_1f95),
                (0x4033_a318_d240_6efa, 0x3fe6_ad05_7939_d43d),
                (0x3fd3_237c_9d81_b740, 0x3fd2_daca_17eb_f845),
                (0xdbc6_8d46_8b48_5879, 0x3fda_813a_639b_d883),
                (0x7fef_ffff_ffff_ffff, 0x3f74_52fc_98b3_4e97),
                (0x40f7_0000_0000_0000, 0xbfeb_eeb9_6dd5_26ca),
                (0x3fb0_0000_0000_0001, 0x3faf_faaa_eeed_4edd),
                (0x0010_0000_0000_0001, 0x0010_0000_0000_0001),
                (0x8000_0000_0000_0000, 0x8000_0000_0000_0000),
            ],
        );
    }

    /// `Math.cos` on HotSpot 25.0.4.1 (x86-64), as for `sin_is_hotspots`; small arguments give 1.
    #[test]
    fn cos_is_hotspots() {
        check(
            "cos",
            super::cos,
            &[
                (0xc024_cc83_a407_94e3, 0xbfe1_f75e_15b9_862f),
                (0xc037_2f94_fca5_1cd5, 0xbfd7_816f_e209_1f3d),
                (0x4021_2f6a_9286_cbd8, 0xbfe5_8b76_9fbf_2f3b),
                (0xbff6_1736_adf2_8cc0, 0x3fc8_30ad_6f9a_4b45),
                (0xc9b3_46b5_9bc1_27ea, 0x3fde_4684_5525_fecd),
                (0x7fef_ffff_ffff_ffff, 0xbfef_ffe6_2ecf_ab75),
                (0x40f7_0000_0000_0000, 0xbfdf_39ff_c707_97c8),
                (0x3e10_0000_0000_0003, 0x3ff0_0000_0000_0000),
                (0x8000_0000_0000_0000, 0x3ff0_0000_0000_0000),
            ],
        );
    }

    /// `Math.tan` on HotSpot 25.0.4.1 (x86-64), as for `sin_is_hotspots`, with the small-argument paths: the
    /// polynomial below 0.1015625, the one below 2^-29 and subnormals.
    #[test]
    fn tan_is_hotspots() {
        check(
            "tan",
            super::tan,
            &[
                (0x4014_b21f_6a33_a538, 0xc000_157a_2f04_f97a),
                (0x4035_7269_74b7_c4e4, 0xbfe3_5db7_6ee0_b0b6),
                (0xc02d_bb0c_c263_fea2, 0x3ff1_f1e5_5a5d_88ff),
                (0x4014_0c0c_703a_461c, 0xc009_eb42_e8a2_29ab),
                (0xebff_def8_83c6_2b05, 0x3fea_dcac_0ba3_3c2f),
                (0x7fef_ffff_ffff_ffff, 0xbf74_530c_fe72_9484),
                (0x40f7_0000_0000_0000, 0x3ffc_9fd6_a0ce_0d4f),
                (0x3fb0_0000_0000_0001, 0x3fb0_0557_7854_df02),
                (0x3e10_0000_0000_0003, 0x3e10_0000_0000_0003),
                (0x0001_0000_0000_0000, 0x0001_0000_0000_0000),
                (0x8000_0000_0000_0000, 0x8000_0000_0000_0000),
            ],
        );
    }

    /// Independent expected bits at dispatch and argument-reduction boundaries.
    #[test]
    fn trig_boundaries_match_expected_bits() {
        let cases: &[(u64, u64, u64, u64)] = &[
            // Zero and subnormals.
            (
                0x0000_0000_0000_0000,
                0x0000_0000_0000_0000,
                0x3ff0_0000_0000_0000,
                0x0000_0000_0000_0000,
            ),
            (
                0x8000_0000_0000_0000,
                0x8000_0000_0000_0000,
                0x3ff0_0000_0000_0000,
                0x8000_0000_0000_0000,
            ),
            (
                0x0000_0000_0000_0001,
                0x0000_0000_0000_0001,
                0x3ff0_0000_0000_0000,
                0x0000_0000_0000_0001,
            ),
            (
                0x8000_0000_0000_0001,
                0x8000_0000_0000_0001,
                0x3ff0_0000_0000_0000,
                0x8000_0000_0000_0001,
            ),
            (
                0x000f_ffff_ffff_ffff,
                0x000f_ffff_ffff_ffff,
                0x3ff0_0000_0000_0000,
                0x000f_ffff_ffff_ffff,
            ),
            (
                0x800f_ffff_ffff_ffff,
                0x800f_ffff_ffff_ffff,
                0x3ff0_0000_0000_0000,
                0x800f_ffff_ffff_ffff,
            ),
            // Near nonzero multiples of π/2.
            (
                0x3ff9_21fb_5444_2d17,
                0x3ff0_0000_0000_0000,
                0x3cb4_6989_8cc5_1702,
                0x4329_153d_9443_ed0b,
            ),
            (
                0x3ff9_21fb_5444_2d18,
                0x3ff0_0000_0000_0000,
                0x3c91_a626_3314_5c07,
                0x434d_0296_7c31_cdb5,
            ),
            (
                0x3ff9_21fb_5444_2d19,
                0x3ff0_0000_0000_0000,
                0xbca7_2cec_e675_d1fd,
                0xc336_17a1_5494_767a,
            ),
            (
                0x4009_21fb_5444_2d17,
                0x3cc4_6989_8cc5_1702,
                0xbff0_0000_0000_0000,
                0xbcc4_6989_8cc5_1702,
            ),
            (
                0x4009_21fb_5444_2d18,
                0x3ca1_a626_3314_5c07,
                0xbff0_0000_0000_0000,
                0xbca1_a626_3314_5c07,
            ),
            (
                0x4009_21fb_5444_2d19,
                0xbcb7_2cec_e675_d1fd,
                0xbff0_0000_0000_0000,
                0x3cb7_2cec_e675_d1fd,
            ),
            (
                0x4012_d97c_7f33_21d1,
                0xbff0_0000_0000_0000,
                0xbcd3_4f27_2993_d141,
                0x430a_8410_0872_62e4,
            ),
            (
                0x4012_d97c_7f33_21d2,
                0xbff0_0000_0000_0000,
                0xbcaa_7939_4c9e_8a0a,
                0x4333_570e_fd76_8923,
            ),
            (
                0x4012_d97c_7f33_21d3,
                0xbff0_0000_0000_0000,
                0x3cc9_61b1_acd8_5d7d,
                0xc314_2c0d_64d5_de51,
            ),
            (
                0x4019_21fb_5444_2d17,
                0xbcd4_6989_8cc5_1702,
                0x3ff0_0000_0000_0000,
                0xbcd4_6989_8cc5_1702,
            ),
            (
                0x4019_21fb_5444_2d18,
                0xbcb1_a626_3314_5c07,
                0x3ff0_0000_0000_0000,
                0xbcb1_a626_3314_5c07,
            ),
            (
                0x4019_21fb_5444_2d19,
                0x3cc7_2cec_e675_d1fd,
                0x3ff0_0000_0000_0000,
                0x3cc7_2cec_e675_d1fd,
            ),
            (
                0x4063_a28c_59d5_433a,
                0xbd1e_e4e6_ebf3_f3f3,
                0x3ff0_0000_0000_0000,
                0xbd1e_e4e6_ebf3_f3f3,
            ),
            (
                0x4063_a28c_59d5_433b,
                0x3cd1_b191_40c0_c0d5,
                0x3ff0_0000_0000_0000,
                0x3cd1_b191_40c0_c0d5,
            ),
            (
                0x4063_a28c_59d5_433c,
                0x3d20_8d8c_8a06_0607,
                0x3ff0_0000_0000_0000,
                0x3d20_8d8c_8a06_0607,
            ),
            (
                0x4139_21fb_5444_2d17,
                0xbdf4_6989_8cc5_1702,
                0x3ff0_0000_0000_0000,
                0xbdf4_6989_8cc5_1702,
            ),
            (
                0x4139_21fb_5444_2d18,
                0xbdd1_a626_3314_5c07,
                0x3ff0_0000_0000_0000,
                0xbdd1_a626_3314_5c07,
            ),
            (
                0x4139_21fb_5444_2d19,
                0x3de7_2cec_e675_d1fd,
                0x3ff0_0000_0000_0000,
                0x3de7_2cec_e675_d1fd,
            ),
            // Dispatch boundaries.
            (
                0x0010_0000_0000_0000,
                0x0010_0000_0000_0000,
                0x3ff0_0000_0000_0000,
                0x0010_0000_0000_0000,
            ),
            (
                0x0010_0000_0000_0001,
                0x0010_0000_0000_0001,
                0x3ff0_0000_0000_0000,
                0x0010_0000_0000_0001,
            ),
            (
                0x302f_ffff_ffff_ffff,
                0x302f_ffff_ffff_ffff,
                0x3ff0_0000_0000_0000,
                0x302f_ffff_ffff_ffff,
            ),
            (
                0x3030_0000_0000_0000,
                0x3030_0000_0000_0000,
                0x3ff0_0000_0000_0000,
                0x3030_0000_0000_0000,
            ),
            (
                0x3030_0000_0000_0001,
                0x3030_0000_0000_0001,
                0x3ff0_0000_0000_0000,
                0x3030_0000_0000_0001,
            ),
            (
                0x3e1f_ffff_ffff_ffff,
                0x3e1f_ffff_ffff_ffff,
                0x3ff0_0000_0000_0000,
                0x3e1f_ffff_ffff_ffff,
            ),
            (
                0x3e20_0000_0000_0000,
                0x3e20_0000_0000_0000,
                0x3ff0_0000_0000_0000,
                0x3e20_0000_0000_0000,
            ),
            (
                0x3e20_0000_0000_0001,
                0x3e20_0000_0000_0001,
                0x3ff0_0000_0000_0000,
                0x3e20_0000_0000_0001,
            ),
            (
                0x3fb9_ffff_ffff_ffff,
                0x3fb9_f490_2d55_d1f8,
                0x3fef_d5c9_4b43_e000,
                0x3fba_16fa_f0d4_0346,
            ),
            (
                0x3fba_0000_0000_0000,
                0x3fb9_f490_2d55_d1f9,
                0x3fef_d5c9_4b43_e000,
                0x3fba_16fa_f0d4_0347,
            ),
            (
                0x3fba_0000_0000_0001,
                0x3fb9_f490_2d55_d1fa,
                0x3fef_d5c9_4b43_e000,
                0x3fba_16fa_f0d4_0348,
            ),
            (
                0x40c8_ffff_ffff_ffff,
                0x3fed_3a78_8da1_2a1b,
                0x3fda_0dc7_d085_0a09,
                0x4001_f315_77f0_d5f5,
            ),
            (
                0x40c9_0000_0000_0000,
                0x3fed_3a78_8da1_4429,
                0x3fda_0dc7_d084_951f,
                0x4001_f315_77f1_3681,
            ),
            (
                0x40c9_0000_0000_0001,
                0x3fed_3a78_8da1_5e36,
                0x3fda_0dc7_d084_2035,
                0x4001_f315_77f1_970d,
            ),
            (
                0x40f5_ffff_ffff_ffff,
                0xbfef_bde6_256f_48ce,
                0x3fc0_3a48_7efe_c87f,
                0xc01f_4be5_3892_ac68,
            ),
            (
                0x40f6_0000_0000_0000,
                0xbfef_bde6_256f_07e5,
                0x3fc0_3a48_7f06_b7f8,
                0xc01f_4be5_3883_1e88,
            ),
            (
                0x40f6_0000_0000_0001,
                0xbfef_bde6_256e_c6fc,
                0x3fc0_3a48_7f0e_a772,
                0xc01f_4be5_3873_90a8,
            ),
            (
                0xbfba_0000_0000_0000,
                0xbfb9_f490_2d55_d1f9,
                0x3fef_d5c9_4b43_e000,
                0xbfba_16fa_f0d4_0347,
            ),
            (
                0xc0c9_0000_0000_0000,
                0xbfed_3a78_8da1_4429,
                0x3fda_0dc7_d084_951f,
                0xc001_f315_77f1_3681,
            ),
            (
                0xc0f6_0000_0000_0000,
                0x3fef_bde6_256f_07e5,
                0x3fc0_3a48_7f06_b7f8,
                0x401f_4be5_3883_1e88,
            ),
            // Large reduction.
            (
                0x4340_0000_0000_0000,
                0xbfeb_2a66_c8f3_5586,
                0xbfe0_e991_8bb3_5aac,
                0x3ff9_b33a_f5ae_241f,
            ),
            (
                0xc340_0000_0000_0000,
                0x3feb_2a66_c8f3_5586,
                0xbfe0_e991_8bb3_5aac,
                0xbff9_b33a_f5ae_241f,
            ),
            (
                0x4480_f0cf_064d_d592,
                0xbfeb_453a_b76b_f397,
                0x3fe0_be2c_ef01_c8f4,
                0xbffa_0f79_c1b6_b257,
            ),
            (
                0xc480_f0cf_064d_d592,
                0x3feb_453a_b76b_f397,
                0x3fe0_be2c_ef01_c8f4,
                0x3ffa_0f79_c1b6_b257,
            ),
        ];
        for &(x, sin, cos, tan) in cases {
            check("sin", super::sin, &[(x, sin)]);
            check("cos", super::cos, &[(x, cos)]);
            check("tan", super::tan, &[(x, tan)]);
        }
    }

    /// Infinities give x86-64's default NaN and a NaN comes back quiet, as `x * -0.0` does in the stubs.
    #[test]
    fn non_finite_arguments_give_x86_nans() {
        for f in [super::sin, super::cos, super::tan] {
            assert_eq!(f(f64::INFINITY).to_bits(), 0xfff8_0000_0000_0000);
            assert_eq!(f(f64::NEG_INFINITY).to_bits(), 0xfff8_0000_0000_0000);
            assert_eq!(
                f(f64::from_bits(0x7ff0_0000_0000_0123)).to_bits(),
                0x7ff8_0000_0000_0123
            );
        }
    }
}
