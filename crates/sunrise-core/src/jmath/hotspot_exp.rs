//! `Math.exp` as HotSpot computes it on x86-64: the stub that the interpreter, C1 and C2 all call.
//!
//! Written from a clean-room description of the stub's algorithm; no HotSpot or OpenJDK source was read. The
//! stored values the description leaves out (the polynomial's coefficients and the table's corrections) were
//! recovered from the JVM's own results on chosen arguments.
//!
//! `x = (64 n + j) ln2/64 + r` with |r| <= ln2/128, and `e^x = 2^n * Thi[j] * (1 + Tlo[j]) * e^r`: `Thi[j]` is
//! 2^(j/64) rounded toward zero, `Tlo[j]` the relative remainder, and e^r - 1 a degree-6 polynomial in `r`. Each
//! step is one rounded `f64` operation in the stub's order, with no fused multiply-add. Outside the normal range
//! the scale is split in two, so that the intermediate values stay normal and a subnormal result is rounded once.
//! The common case, |x| < 619, takes one comparison and one table entry; every other argument goes through the
//! stub's paths in order (`exp_far`).

/// RN(64 / ln 2).
const C: f64 = f64::from_bits(0x4057_1547_652b_82fe);
/// 1.5 * 2^52: adding it rounds to an integer.
const SHIFTER: f64 = f64::from_bits(0x4338_0000_0000_0000);
/// ln2/64 to 36 significant bits, so `m * LN2_64_HI` is exact for every `m` the main path meets.
const LN2_64_HI: f64 = f64::from_bits(0x3f86_2e42_fefa_0000);
/// RN(ln2/64 - LN2_64_HI).
const LN2_64_LO: f64 = f64::from_bits(0x3d1c_f79a_bc9e_3b3a);

/// e^r - 1 = r + C2 r^2 + ... + C6 r^6 on |r| <= ln2/128.
const C2: f64 = f64::from_bits(0x3fdf_ffff_ffff_fffe);
const C3: f64 = f64::from_bits(0x3fc5_5555_5554_8ba1);
const C4: f64 = f64::from_bits(0x3fa5_5555_555b_9e25);
const C5: f64 = f64::from_bits(0x3f81_1115_c090_cf0f);
const C6: f64 = f64::from_bits(0x3f56_c15c_e328_9860);

/// 2^(j/64) rounded toward zero.
const THI: [u64; 64] = [
    0x3ff0_0000_0000_0000,
    0x3ff0_2c9a_3e77_8060,
    0x3ff0_59b0_d315_8574,
    0x3ff0_8745_1875_9bc8,
    0x3ff0_b558_6cf9_890f,
    0x3ff0_e3ec_32d3_d1a2,
    0x3ff1_1301_d012_5b50,
    0x3ff1_429a_aea9_2ddf,
    0x3ff1_72b8_3c7d_517a,
    0x3ff1_a35b_eb6f_cb75,
    0x3ff1_d487_3168_b9aa,
    0x3ff2_063b_8862_8cd6,
    0x3ff2_387a_6e75_6238,
    0x3ff2_6b45_65e2_7cdd,
    0x3ff2_9e9d_f51f_dee1,
    0x3ff2_d285_a6e4_030b,
    0x3ff3_06fe_0a31_b715,
    0x3ff3_3c08_b264_16ff,
    0x3ff3_71a7_373a_a9ca,
    0x3ff3_a7db_34e5_9ff6,
    0x3ff3_dea6_4c12_3422,
    0x3ff4_160a_21f7_2e29,
    0x3ff4_4e08_6061_892d,
    0x3ff4_86a2_b5c1_3cd0,
    0x3ff4_bfda_d536_2a27,
    0x3ff4_f9b2_769d_2ca6,
    0x3ff5_342b_569d_4f81,
    0x3ff5_6f47_36b5_27da,
    0x3ff5_ab07_dd48_5429,
    0x3ff5_e76f_15ad_2148,
    0x3ff6_247e_b03a_5584,
    0x3ff6_6238_8255_2224,
    0x3ff6_a09e_667f_3bcc,
    0x3ff6_dfb2_3c65_1a2e,
    0x3ff7_1f75_e8ec_5f73,
    0x3ff7_5feb_5642_67c8,
    0x3ff7_a114_73eb_0186,
    0x3ff7_e2f3_36cf_4e62,
    0x3ff8_2589_994c_ce12,
    0x3ff8_68d9_9b44_92ec,
    0x3ff8_ace5_422a_a0db,
    0x3ff8_f1ae_9915_7736,
    0x3ff9_3737_b0cd_c5e4,
    0x3ff9_7d82_9fde_4e4f,
    0x3ff9_c491_82a3_f090,
    0x3ffa_0c66_7b5d_e564,
    0x3ffa_5503_b23e_255c,
    0x3ffa_9e6b_5579_fdbf,
    0x3ffa_e89f_995a_d3ad,
    0x3ffb_33a2_b84f_15fa,
    0x3ffb_7f76_f2fb_5e46,
    0x3ffb_cc1e_904b_c1d2,
    0x3ffc_199b_dd85_529c,
    0x3ffc_67f1_2e57_d14b,
    0x3ffc_b720_dcef_9069,
    0x3ffd_072d_4a07_897b,
    0x3ffd_5818_dcfb_a487,
    0x3ffd_a9e6_03db_3285,
    0x3ffd_fc97_337b_9b5e,
    0x3ffe_502e_e78b_3ff6,
    0x3ffe_a4af_a2a4_90d9,
    0x3ffe_fa1b_ee61_5a27,
    0x3fff_5076_5b6e_4540,
    0x3fff_a7c1_819e_90d8,
];

/// RN((2^(j/64) - Thi[j]) / Thi[j]).
const TLO: [u64; 64] = [
    0x0000_0000_0000_0000,
    0x3cad_7bbf_0e03_754e,
    0x3c8c_d252_3567_f613,
    0x3c60_f74e_61e6_c861,
    0x3c97_9aa6_5d83_7b6d,
    0x3c3e_be3d_702f_9cd1,
    0x3ca3_516e_1e63_bcd8,
    0x3ca4_c554_26f0_387b,
    0x3ca9_5153_6252_3fb6,
    0x3c8b_898c_3f13_53bf,
    0x3c9a_ecf7_3e3a_2f60,
    0x3c8a_6f41_44a6_c38d,
    0x3c96_8efd_e3a8_a894,
    0x3c80_472b_981f_e7f2,
    0x3c82_f7e1_6d09_ab31,
    0x3c8b_3782_720c_0ab4,
    0x3c83_4d75_4db0_abb6,
    0x3c8f_dd39_5dd3_f84a,
    0x3ca1_2f8c_cc18_7d29,
    0x3ca7_d229_738b_5e8c,
    0x3c85_9f48_a72a_4c6d,
    0x3ca8_b846_259d_9206,
    0x3c43_63ed_60c2_ac11,
    0x3c6e_cce1_daa1_0379,
    0x3c76_90ce_bb7a_afb0,
    0x3ca0_83cc_9b28_2a09,
    0x3ca5_09b0_c1aa_e707,
    0x3c93_3505_18fd_d78e,
    0x3c90_63e1_e21c_5409,
    0x3c94_32e6_2b64_c035,
    0x3ca0_1284_99f0_8c0a,
    0x3c99_f087_0073_dc06,
    0x3c99_8d4d_0da0_5571,
    0x3ca5_2bb9_86ce_4787,
    0x3ca3_2092_206f_0dac,
    0x3ca0_6122_8e17_a7a6,
    0x3ca2_44ac_461e_9f86,
    0x3c65_ebe1_abd6_6c55,
    0x3c96_fe9f_bbff_67d0,
    0x3c95_1f14_14c8_01df,
    0x3c8d_b72f_c1f0_eab4,
    0x3c7b_f683_59f3_5f44,
    0x3ca3_60ba_9c06_283c,
    0x3c95_e8d1_20f9_62aa,
    0x3c71_affc_2b91_ce27,
    0x3c9b_6d34_589a_2ebd,
    0x3c95_277c_9ab8_9881,
    0x3c84_6984_6e73_5ab3,
    0x3c8c_1a77_92cb_3387,
    0x3ca2_2466_dc2d_1d96,
    0x3ca1_112e_b195_05ae,
    0x3c74_ffd7_0a5f_ddcd,
    0x3c73_6eae_30af_0cb3,
    0x3c84_e08f_d109_59ac,
    0x3c67_6b2c_6c92_1968,
    0x3c93_7009_36df_99b3,
    0x3c74_a385_a63d_07a7,
    0x3c8e_5a50_d5c1_92ac,
    0x3c98_bb73_1c4a_9792,
    0x3c74_b604_603a_88d3,
    0x3c91_6f27_9209_4927,
    0x3c8e_c3bc_41aa_2008,
    0x3c8a_64a9_31d1_85ee,
    0x3c77_893b_4d91_cd9d,
];

const FRACTION: u64 = 0x000f_ffff_ffff_ffff;

/// Thi[j]'s fraction bits and Tlo[j] side by side, so one entry is one 16-byte load.
#[repr(C, align(16))]
struct Entry {
    thi_fraction: u64,
    tlo: f64,
}

static TABLE: [Entry; 64] = {
    let mut table = [const {
        Entry {
            thi_fraction: 0,
            tlo: 0.0,
        }
    }; 64];
    let mut j = 0;
    while j < 64 {
        table[j] = Entry {
            thi_fraction: THI[j] & FRACTION,
            tlo: f64::from_bits(TLO[j]),
        };
        j += 1;
    }
    table
};

/// |x| < 619, as bits: n is then in -894..=1022, the normal range. Arguments the stub sends to its tiny-argument path
/// (|x| < 2^-54, zeros, subnormals) take the main path here and get the same 1: there r = x, q = x (or 0) and
/// q * 1 + 1 rounds to 1.
const NORMAL_LIMIT: u64 = 0x4083_5800_0000_0000;

/// 2^e for a normal result.
fn pow2(e: i32) -> f64 {
    f64::from_bits(((e + 1023) as u64) << 52)
}

/// `Math.exp(x)` on x86-64 HotSpot.
#[inline]
pub fn exp(x: f64) -> f64 {
    if x.to_bits() & !(1 << 63) >= NORMAL_LIMIT {
        return exp_far(x);
    }
    let k = x * C + SHIFTER;
    let m = k.to_bits() as u32 as i32;
    let md = k - SHIFTER;
    let r = (x - md * LN2_64_HI) - md * LN2_64_LO;
    let entry = &TABLE[(m & 63) as usize];
    let n = m >> 6;
    let r2 = r * r;
    let r3 = r * r2;
    let r5 = r3 * r2;
    let a = C5 + C6 * r;
    let b = C3 + C4 * r;
    let mut u = r + entry.tlo;
    u += r5 * a;
    let v = r3 * b + u;
    let q = v + C2 * r2;
    let s = f64::from_bits(entry.thi_fraction | (((n + 1023) as u64) << 52));
    q * s + s
}

/// The stub's every path in its own order: the special arguments, then the normal and the split-scale reconstruction.
#[cold]
#[inline(never)]
fn exp_far(x: f64) -> f64 {
    // The stub sorts its argument by the top 16 bits of |x|.
    let top = (x.to_bits() >> 48) & 0x7fff;
    if top < 0x3c90 {
        // |x| < 2^-54, zeros and subnormals: 1.
        return x + 1.0;
    }
    if top >= 0x4090 {
        // |x| >= 1024, infinities and NaN.
        return if x.is_nan() {
            x + x
        } else if x == f64::INFINITY {
            x
        } else if x == f64::NEG_INFINITY {
            0.0
        } else if x > 0.0 {
            f64::MAX * f64::MAX
        } else {
            f64::MIN_POSITIVE * f64::MIN_POSITIVE
        };
    }
    // m = x 64/ln2 rounded to an integer (ties to even), j = m mod 64, n = floor(m / 64).
    let k = x * C + SHIFTER;
    let m = k.to_bits() as u32 as i32;
    let md = k - SHIFTER;
    let r = (x - md * LN2_64_HI) - md * LN2_64_LO;
    let j = (m & 63) as usize;
    let n = m >> 6;
    // q ~ e^r (1 + Tlo[j]) - 1, in the stub's order.
    let r2 = r * r;
    let r3 = r * r2;
    let r5 = r3 * r2;
    let a = C5 + C6 * r;
    let b = C3 + C4 * r;
    let mut u = r + TABLE[j].tlo;
    u += r5 * a;
    let v = r3 * b + u;
    let q = v + C2 * r2;
    if (-894..=1022).contains(&n) {
        let s = f64::from_bits(TABLE[j].thi_fraction | (((n + 1023) as u64) << 52));
        return q * s + s;
    }
    // Far from 1: the scale as Thi[j] 2^n2 times 2^n1, both normal.
    let n1 = n >> 1;
    let n2 = n - n1;
    let s2 = f64::from_bits(TABLE[j].thi_fraction | (((n2 + 1023) as u64) << 52));
    let p = pow2(n1);
    let qs = q * s2;
    if n < -1074 {
        return (qs + s2) * p;
    }
    // H keeps the bits of S2 that survive on the subnormal grid; L is the rest, exactly.
    let shift = -1022 - n;
    let h = if shift < 0 {
        0.0
    } else {
        f64::from_bits(s2.to_bits() & !((1u64 << shift) - 1))
    };
    let low = qs + (s2 - h);
    if n >= 1023 || (n == -1022 && low.is_sign_positive()) {
        return (low + h) * p;
    }
    let rounded = (low + h) * p;
    if rounded.to_bits() & !FRACTION & !(1 << 63) != 0 {
        return rounded;
    }
    // A subnormal or zero result: the low part rounded once onto the subnormal grid, then added to the exact high
    // part as integers (a carry out of the fraction gives the smallest normal).
    let a = low * p;
    let b = h * p;
    let mut magnitude = a.to_bits() & !(1 << 63);
    if a.is_sign_negative() != b.is_sign_negative() {
        magnitude = magnitude.wrapping_neg();
    }
    f64::from_bits(b.to_bits().wrapping_add(magnitude))
}

#[cfg(test)]
mod tests {
    /// `Math.exp` on HotSpot 25.0.4.1 (x86-64): input and result bits. The first four are arguments where the stub's
    /// result is not the correctly rounded one (glibc's `exp` gives the other neighbour); then 1 for tiny arguments,
    /// the infinities and the limits at 1024, the overflow edge, the normal and subnormal results around n = -1022
    /// and the deepest underflow.
    #[test]
    fn exp_is_hotspots() {
        for (x, expected) in [
            (0xc036_5673_6bde_53b4_u64, 0x3deb_59d0_3359_224e_u64),
            (0x4033_23fe_541c_2aa4, 0x41a8_7d1f_8bb1_2f3a),
            (0x401c_c63e_88ba_bc40, 0x4094_cb88_35ac_4276),
            (0x4038_f574_c8b6_6988, 0x4230_16a6_972f_f622),
            (0x3c67_0ef5_4646_d497, 0x3ff0_0000_0000_0000),
            (0x8000_0000_0000_0000, 0x3ff0_0000_0000_0000),
            (0x0000_0000_0000_0001, 0x3ff0_0000_0000_0000),
            (0x7ff0_0000_0000_0000, 0x7ff0_0000_0000_0000),
            (0xfff0_0000_0000_0000, 0x0000_0000_0000_0000),
            (0x4090_0000_0000_0000, 0x7ff0_0000_0000_0000),
            (0xc090_0000_0000_0000, 0x0000_0000_0000_0000),
            (0x4086_2e42_fefa_39ef, 0x7fef_ffff_ffff_ff2a),
            (0x4086_2e42_fefa_39f0, 0x7ff0_0000_0000_0000),
            (0xc086_232b_dd7a_bcd2, 0x0010_0000_0000_007c),
            (0xc086_232b_dd7a_bcae, 0x0010_0000_0000_487c),
            (0xc086_2400_0000_0000, 0x000e_6cf6_d088_97ac),
            (0xc086_80fc_d6e9_b9cb, 0x0000_0008_76f7_02fc),
            (0xc087_4385_1eb8_51ec, 0x0000_0000_0000_0001),
            (0xc087_4910_d52d_3051, 0x0000_0000_0000_0001),
            (0xc087_4910_d52d_3052, 0x0000_0000_0000_0000),
            (0xbf50_624d_d2f1_a9fc, 0x3fef_f7cf_e56f_1a9e),
            (0x3fe0_0000_0000_0000, 0x3ffa_6129_8e1e_069c),
            (0xc034_4000_0000_0000, 0x3e1b_93de_1e27_ca3b),
        ] {
            assert_eq!(
                super::exp(f64::from_bits(x)).to_bits(),
                expected,
                "exp({x:#x})"
            );
        }
        // A NaN keeps its payload and sign and comes back quiet (x + x on x86-64).
        if cfg!(target_arch = "x86_64") {
            for (x, expected) in [
                (0x7ff0_0000_0000_0123_u64, 0x7ff8_0000_0000_0123_u64),
                (0xfff8_0000_0000_0456, 0xfff8_0000_0000_0456),
            ] {
                assert_eq!(
                    super::exp(f64::from_bits(x)).to_bits(),
                    expected,
                    "exp({x:#x})"
                );
            }
        }
    }
}
