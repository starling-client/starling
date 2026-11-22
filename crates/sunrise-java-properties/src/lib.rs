//! `java.util.Properties` as the dedicated server uses it: `load` (the
//! LineReader and loadConvert of the JDK), `store` with the sorted entries the
//! JDK writes since Java 18, and `java.util.Date.toString` for the timestamp
//! comment. Values are kept in a map keyed like Java's Hashtable; iteration
//! for storing sorts by `String.compareTo` (UTF-16 code units).
use std::collections::HashMap;

/// A malformed `\uxxxx` escape: Properties.load throws IllegalArgumentException.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Malformed;
impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Malformed \\uxxxx encoding.")
    }
}
impl std::error::Error for Malformed {}

/// An exact Java property key or value, including any unpaired surrogates.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JavaString(Box<[u16]>);

impl JavaString {
    pub fn as_units(&self) -> &[u16] {
        &self.0
    }

    /// Convert at a boundary that cannot represent unpaired surrogates.
    pub fn to_utf8(&self) -> Result<String, std::string::FromUtf16Error> {
        String::from_utf16(&self.0)
    }

    pub fn has_unpaired_surrogate(&self) -> bool {
        char::decode_utf16(self.0.iter().copied()).any(|unit| unit.is_err())
    }

    /// The distinct key that reloading the UTF-8 Writer form would yield, if encoding changes it.
    pub fn utf8_writer_alias(&self) -> Option<String> {
        self.has_unpaired_surrogate().then(|| {
            char::decode_utf16(self.0.iter().copied())
                .map(|unit| unit.unwrap_or(UTF8_WRITER_REPLACEMENT))
                .collect()
        })
    }
}

const UTF8_WRITER_REPLACEMENT: char = '?';

impl From<&str> for JavaString {
    fn from(text: &str) -> Self {
        if text.is_ascii() {
            Self(text.bytes().map(u16::from).collect())
        } else {
            Self(text.encode_utf16().collect())
        }
    }
}

impl std::borrow::Borrow<[u16]> for JavaString {
    fn borrow(&self) -> &[u16] {
        &self.0
    }
}

/// Settings keys normally fit in the stack buffer; still allow arbitrary keys.
fn with_key<R>(key: &str, use_units: impl FnOnce(&[u16]) -> R) -> R {
    let mut units = [0; 64];
    if key.len() <= units.len() {
        let mut len = 0;
        for unit in key.encode_utf16() {
            units[len] = unit;
            len += 1;
        }
        use_units(&units[..len])
    } else {
        use_units(JavaString::from(key).as_units())
    }
}

/// The key/value pairs, last occurrence winning (Hashtable.put).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Properties {
    map: HashMap<JavaString, JavaString>,
}

impl Properties {
    /// Text for Rust consumers. Unpaired units become U+FFFD here only.
    pub fn get(&self, key: &str) -> Option<String> {
        self.get_java(key)
            .map(|text| String::from_utf16_lossy(text.as_units()))
    }
    pub fn get_java(&self, key: &str) -> Option<&JavaString> {
        with_key(key, |key| self.map.get(key))
    }
    pub fn iter(&self) -> impl Iterator<Item = (&JavaString, &JavaString)> {
        self.map.iter()
    }
    pub fn put(&mut self, key: &str, value: String) {
        self.map
            .insert(JavaString::from(key), JavaString::from(value.as_str()));
    }
    pub fn remove(&mut self, key: &str) -> Option<String> {
        with_key(key, |key| self.map.remove(key))
            .map(|text| String::from_utf16_lossy(text.as_units()))
    }

    /// Properties.load(Reader) over an already decoded UTF-8 string.
    pub fn load_str(text: &str) -> Result<Self, Malformed> {
        // Typical settings files have dozens of short rows. Avoid repeatedly
        // rehashing the keys while bounding the guess for long comments/values.
        let mut out = Self {
            map: HashMap::with_capacity((text.len() / 24).min(64)),
        };
        let mut reader = LineReader(text);
        let mut scratch = String::new();
        while let Some(line) = reader.read_line(&mut scratch) {
            let bytes = line.as_bytes();
            let mut key_len = 0;
            let mut value_start = line.len();
            let mut has_sep = false;
            let mut preceding_backslash = false;
            while key_len < line.len() {
                let c = bytes[key_len];
                if (c == b'=' || c == b':') && !preceding_backslash {
                    value_start = key_len + 1;
                    has_sep = true;
                    break;
                } else if (c == b' ' || c == b'\t' || c == b'\x0c') && !preceding_backslash {
                    value_start = key_len + 1;
                    break;
                }
                if c == b'\\' {
                    preceding_backslash = !preceding_backslash;
                } else {
                    preceding_backslash = false;
                }
                key_len += 1;
            }
            while value_start < line.len() {
                let c = bytes[value_start];
                if c != b' ' && c != b'\t' && c != b'\x0c' {
                    if !has_sep && (c == b'=' || c == b':') {
                        has_sep = true;
                    } else {
                        break;
                    }
                }
                value_start += 1;
            }
            let key = load_convert(&line[..key_len])?;
            let value = load_convert(&line[value_start..])?;
            out.map.insert(key, value);
        }
        Ok(out)
    }

    /// Settings.loadFromFile: UTF-8 with errors reported, falling back to
    /// ISO-8859-1 for the whole file. A missing file is an empty set.
    pub fn load_file(bytes: &[u8]) -> Result<Self, Malformed> {
        match std::str::from_utf8(bytes) {
            Ok(s) => Self::load_str(s),
            Err(_) => Self::load_latin1(bytes),
        }
    }

    /// Properties.load(InputStream): ISO-8859-1.
    pub fn load_latin1(bytes: &[u8]) -> Result<Self, Malformed> {
        Self::load_str(&bytes.iter().copied().map(char::from).collect::<String>())
    }

    /// Properties.store(Writer, comments): comment, date, then the entries
    /// sorted by key, with `escUnicode` off (the Writer variant) or on (the
    /// OutputStream variant, whose bytes are ISO-8859-1).
    pub fn store(&self, comment: Option<&str>, date: &str, esc_unicode: bool) -> String {
        let nl = line_separator();
        let mut out = String::new();
        if let Some(c) = comment {
            write_comments(&mut out, c, nl);
        }
        out.push('#');
        out.push_str(date);
        out.push_str(nl);
        let mut entries: Vec<_> = self.map.iter().collect();
        entries.sort_unstable_by_key(|(key, _)| *key);
        for (key, value) in entries {
            save_convert(&mut out, key, true, esc_unicode);
            out.push('=');
            save_convert(&mut out, value, false, esc_unicode);
            out.push_str(nl);
        }
        out
    }
}

/// System.lineSeparator().
pub fn line_separator() -> &'static str {
    if cfg!(windows) { "\r\n" } else { "\n" }
}

struct LineReader<'a>(&'a str);

impl<'a> LineReader<'a> {
    /// Return an ordinary logical line without copying it. Continued lines
    /// join physical lines into the reused scratch buffer.
    fn read_line<'b>(&mut self, scratch: &'b mut String) -> Option<&'b str>
    where
        'a: 'b,
    {
        'seek: loop {
            self.0 = self.0.trim_start_matches([' ', '\t', '\u{c}', '\r', '\n']);
            if self.0.is_empty() {
                return None;
            }
            scratch.clear();
            loop {
                let end = self
                    .0
                    .as_bytes()
                    .iter()
                    .position(|&b| matches!(b, b'\r' | b'\n'))
                    .unwrap_or(self.0.len());
                let (line, ending) = self.0.split_at(end);
                // An ending, if present, starts with an ASCII CR or LF.
                self.0 = ending.get(1..).unwrap_or_default();
                if scratch.is_empty() && (line.is_empty() || line.starts_with(['#', '!'])) {
                    continue 'seek;
                }
                let slashes = line
                    .as_bytes()
                    .iter()
                    .rev()
                    .take_while(|&&b| b == b'\\')
                    .count();
                if slashes % 2 == 0 {
                    if scratch.is_empty() {
                        return Some(line);
                    }
                    scratch.push_str(line);
                    return Some(scratch);
                }
                scratch.push_str(&line[..line.len() - 1]);
                if self.0.is_empty() {
                    return Some(scratch);
                }
                if ending.starts_with('\r') {
                    self.0 = self.0.strip_prefix('\n').unwrap_or(self.0);
                }
                self.0 = self.0.trim_start_matches([' ', '\t', '\u{c}']);
            }
        }
    }
}

fn load_convert(input: &str) -> Result<JavaString, Malformed> {
    if !input.contains('\\') {
        return Ok(JavaString::from(input));
    }
    let mut units = input.encode_utf16();
    let mut out = Vec::with_capacity(input.len());
    while let Some(c) = units.next() {
        if c != u16::from(b'\\') {
            out.push(c);
            continue;
        }
        let Some(next) = units.next() else {
            break;
        };
        out.push(match next {
            0x75 => {
                let mut value = 0u16;
                for _ in 0..4 {
                    let digit = units
                        .next()
                        .and_then(|unit| char::from_u32(u32::from(unit)))
                        .and_then(|c| c.to_digit(16))
                        .ok_or(Malformed)?;
                    value = value * 16 + digit as u16;
                }
                value
            }
            0x74 => 0x09,
            0x72 => 0x0d,
            0x6e => 0x0a,
            0x66 => 0x0c,
            other => other,
        });
    }
    Ok(JavaString(out.into_boxed_slice()))
}

fn save_convert(out: &mut String, text: &JavaString, escape_space: bool, esc_unicode: bool) {
    use std::fmt::Write as _;
    // The Writer emits exact code units, then Java's UTF-8 encoder replaces
    // an unpaired surrogate with '?'. OutputStream escapes each unit instead.
    let chars = char::decode_utf16(text.as_units().iter().copied());
    for (index, c) in chars.enumerate() {
        let c = match c {
            Ok(c) => c,
            Err(e) if esc_unicode => {
                write!(out, "\\u{:04X}", e.unpaired_surrogate()).unwrap();
                continue;
            }
            Err(_) => UTF8_WRITER_REPLACEMENT,
        };
        match c {
            ' ' => {
                if index == 0 || escape_space {
                    out.push('\\');
                }
                out.push(' ');
            }
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            '\\' | '=' | ':' | '#' | '!' => {
                out.push('\\');
                out.push(c);
            }
            _ if esc_unicode && !(0x20..=0x7e).contains(&(c as u32)) => {
                for unit in c.encode_utf16(&mut [0; 2]) {
                    write!(out, "\\u{unit:04X}").unwrap();
                }
            }
            _ => out.push(c),
        }
    }
}

/// Properties.writeComments.
fn write_comments(out: &mut String, comments: &str, nl: &str) {
    out.push('#');
    let units: Vec<u16> = comments.encode_utf16().collect();
    let mut current = 0;
    let mut last = 0;
    let len = units.len();
    while current < len {
        let c = units[current];
        if c > 0x00ff || c == u16::from(b'\n') || c == u16::from(b'\r') {
            if last != current {
                out.push_str(&String::from_utf16_lossy(&units[last..current]));
            }
            if c > 0x00ff {
                out.push_str(&format!("\\u{c:04X}"));
            } else {
                out.push_str(nl);
                if c == u16::from(b'\r')
                    && current != len - 1
                    && units[current + 1] == u16::from(b'\n')
                {
                    current += 1;
                }
                if current == len - 1
                    || units[current + 1] != u16::from(b'#')
                        && units[current + 1] != u16::from(b'!')
                {
                    out.push('#');
                }
            }
            last = current + 1;
        }
        current += 1;
    }
    if last != current {
        out.push_str(&String::from_utf16_lossy(&units[last..current]));
    }
    out.push_str(nl);
}

/// Civil date from days since 1970-01-01 (proleptic Gregorian).
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
/// Days since 1970-01-01 of a civil date.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Milliseconds since the epoch, now.
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// java.util.Date.toString in UTC: "EEE MMM dd HH:mm:ss zzz yyyy".
pub fn date_string(millis: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let secs = millis.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{} {} {:02} {:02}:{:02}:{:02} UTC {}",
        DAYS[days.rem_euclid(7) as usize],
        MONTHS[m as usize - 1],
        d,
        rem / 3600,
        rem / 60 % 60,
        rem % 60,
        y
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn load(s: &str) -> Properties {
        Properties::load_str(s).unwrap()
    }
    #[test]
    fn parses_like_java() {
        let p = load(
            "# comment\n  ! other\nkey=value\na:b\nc d\n e  =  f g \nmulti=one\\\n    two\nesc=\\u00e9\\t\\x\nempty\nlast=1\nlast=2\r\ncr=1\rnext=2",
        );
        for (key, value) in [
            ("key", "value"),
            ("a", "b"),
            ("c", "d"),
            ("e", "f g "),
            ("multi", "onetwo"),
            ("esc", "é\tx"),
            ("empty", ""),
            ("last", "2"),
            ("cr", "1"),
            ("next", "2"),
        ] {
            assert_eq!(p.get(key).as_deref(), Some(value), "{key}");
        }
    }
    #[test]
    fn stores_sorted_and_escaped() {
        let mut p = Properties::default();
        p.put("level-type", "minecraft:normal".into());
        p.put("motd", " A=B#".into());
        p.put("b key", "é".into());
        let s = p.store(Some("Minecraft server properties"), "DATE", false);
        // Each line ends in System.lineSeparator(), as BufferedWriter.newLine writes it.
        assert_eq!(
            s,
            "#Minecraft server properties\n#DATE\nb\\ key=é\nlevel-type=minecraft\\:normal\nmotd=\\ A\\=B\\#\n"
                .replace('\n', line_separator())
        );
        assert!(p.store(None, "D", true).contains("b\\ key=\\u00E9"));
    }
    #[test]
    fn a_nul_and_a_non_bmp_character_are_written_as_java_writes_them() {
        // `\u0000` is a character of the value, not a marker of any kind, and the Writer variant
        // (escUnicode off) writes it as it is.
        let p = load("k=\\u0000\\uD83D\\uDE00x");
        assert_eq!(p.get("k").as_deref(), Some("\u{0}\u{1F600}x"));
        assert_eq!(
            p.store(None, "D", false),
            format!("#D{nl}k=\u{0}\u{1F600}x{nl}", nl = line_separator())
        );
        // With escapes on, the NUL and both halves of the pair are `\u` escapes, upper-case hex.
        assert!(
            p.store(None, "D", true)
                .contains("k=\\u0000\\uD83D\\uDE00x")
        );
    }
    #[test]
    fn surrogate_escapes_roundtrip_in_latin1_and_use_utf8_writer_replacement() {
        // Temurin 25.0.5 Properties.load keeps both keys; its Latin-1 store
        // emits the original units, while an OutputStreamWriter(UTF_8) emits '?'.
        let p = load("?=literal\n\\uD800=\\uDC00x\\uD801\n");
        let nl = line_separator();
        assert_eq!(p.get("?").as_deref(), Some("literal"));
        let latin1 = p.store(None, "D", true);
        assert_eq!(
            latin1,
            format!("#D{nl}?=literal{nl}\\uD800=\\uDC00x\\uD801{nl}")
        );
        assert_eq!(Properties::load_latin1(latin1.as_bytes()).unwrap(), p);
        assert_eq!(
            p.store(None, "D", false),
            format!("#D{nl}?=literal{nl}?=?x?{nl}")
        );
    }

    #[test]
    fn the_crate_has_no_dependency_beyond_the_standard_library() {
        // Keep this small parser free of dependencies.
        let manifest = include_str!("../Cargo.toml");
        let mut table = "";
        for line in manifest.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                table = name;
            } else if table.contains("dependencies") && !line.is_empty() && !line.starts_with('#') {
                panic!("Cargo.toml lists a dependency in [{table}]: {line}");
            }
        }
    }
    #[test]
    fn dates() {
        assert_eq!(date_string(0), "Thu Jan 01 00:00:00 UTC 1970");
        assert_eq!(days_from_civil(2026, 10, 12), 20_738);
        assert_eq!(civil_from_days(20_738), (2026, 10, 12));
    }
}
