//! The anvil's name checks, for the anvil name parity harness: each name in the file given (UTF-16 units in hex,
//! `-` for the empty name) filtered as `StringUtil.filterText` does and tested with `StringUtil.isBlank`; then
//! every UTF-16 unit with `StringUtil.isWhitespace`, `isAllowedChatCharacter` and `isBlank` of the unit alone
//! (`-` for a lone surrogate, which a Rust string can't hold).
use sunrise_client::clipboard::{filter_text, is_allowed_chat_character};
use sunrise_core::java_character::{is_blank, is_space_char, is_whitespace};

fn hex(text: &str) -> String {
    let units: Vec<String> = text.encode_utf16().map(|u| format!("{u:04x}")).collect();
    if units.is_empty() {
        "-".into()
    } else {
        units.join(" ")
    }
}

pub fn main() {
    let path = super::args()
        .nth(1)
        .expect("usage: anvil_name_fixture NAMES");
    for line in std::fs::read_to_string(path).expect("names").lines() {
        let units: Vec<u16> = match line {
            "-" => Vec::new(),
            _ => line
                .split(' ')
                .map(|h| u16::from_str_radix(h, 16).expect("hex unit"))
                .collect(),
        };
        let name = String::from_utf16(&units).expect("a name of whole characters");
        let filtered = filter_text(&name, false);
        println!(
            "n {} | {} {}",
            hex(&name),
            hex(&filtered),
            u8::from(is_blank(&filtered))
        );
    }
    for unit in 0..=u16::MAX {
        let whitespace = u8::from(is_whitespace(unit) || is_space_char(unit));
        match char::from_u32(u32::from(unit)) {
            Some(c) => println!(
                "u {unit:04x} {whitespace} {} {}",
                u8::from(is_allowed_chat_character(c)),
                u8::from(is_blank(c.encode_utf8(&mut [0; 4])))
            ),
            None => println!("u {unit:04x} {whitespace} - -"),
        }
    }
}
