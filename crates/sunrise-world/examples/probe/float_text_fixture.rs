//! Java numeric text, actual SNBT and DataFixer numeric text on the same bits.
use std::io::Write;
use sunrise_core::nbt::Tag;
use sunrise_world::snbt_text;

pub fn main() {
    let args: Vec<_> = super::args().collect();
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[2]).unwrap());
    for line in std::fs::read_to_string(&args[1]).unwrap().lines() {
        let (kind, bits) = line.split_once(' ').unwrap();
        if kind == "f" {
            let bits = u32::from_str_radix(bits, 16).unwrap();
            let value = f32::from_bits(bits);
            // Vanilla's NBT valueOf factory normalizes both zero signs.
            let tag = Tag::Float(if value == 0.0 { 0 } else { bits });
            writeln!(
                out,
                "{line}\t{}\t{}\t{}",
                sunrise_core::java_number::float_to_string(value),
                snbt_text::to_snbt(&tag),
                sunrise_core::java_number::float_to_string(value)
            )
            .unwrap();
        } else {
            let bits = u64::from_str_radix(bits, 16).unwrap();
            let value = f64::from_bits(bits);
            let tag = Tag::Double(if value == 0.0 { 0 } else { bits });
            writeln!(
                out,
                "{line}\t{}\t{}\t{}",
                sunrise_core::java_number::double_to_string(value),
                snbt_text::to_snbt(&tag),
                sunrise_core::java_number::double_to_string(value)
            )
            .unwrap();
        }
    }
}
