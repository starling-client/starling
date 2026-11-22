//! `SnbtGrammar`/`TagParser` over NbtOps, shared by commands and persistent codecs.
use super::{Compound, JavaString, Tag};
use std::ops::Range;

/// A grammar failure, with an input-relative cursor and its completion suppliers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnbtError {
    pub key: &'static str,
    pub args: Vec<String>,
    pub cursor: usize,
    pub suggestions: Vec<String>,
}
impl std::fmt::Display for SnbtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.key, self.cursor)?;
        for arg in &self.args {
            write!(f, ": {arg}")?;
        }
        Ok(())
    }
}
impl std::error::Error for SnbtError {}

#[derive(Clone, Copy)]
enum Input<'a> {
    Utf8(&'a str),
    Utf16(&'a [u16]),
}
impl<'a> Input<'a> {
    fn at(self, cursor: usize) -> Option<u32> {
        match self {
            Self::Utf8(text) => text.get(cursor..)?.chars().next().map(u32::from),
            Self::Utf16(text) => text.get(cursor).copied().map(u32::from),
        }
    }
    fn width(self, value: u32) -> usize {
        match self {
            Self::Utf8(_) => char::from_u32(value).expect("UTF-8 scalar").len_utf8(),
            Self::Utf16(_) => 1,
        }
    }
    /// The scanner has already restricted every unit in the span to ASCII digits or underscores.
    fn clean_digits(self, span: Range<usize>) -> impl Iterator<Item = u8> + use<'a> {
        let (bytes, units) = match self {
            Self::Utf8(text) => (&text.as_bytes()[span], &[][..]),
            Self::Utf16(text) => (&[][..], &text[span]),
        };
        bytes
            .iter()
            .copied()
            .chain(units.iter().map(|&unit| unit as u8))
            .filter(|&byte| byte != b'_')
    }
}

/// `TagParser.parseAsArgument`: one value, preserving a byte cursor for UTF-8.
pub fn parse_snbt_argument(text: &str) -> Result<(Tag, usize), SnbtError> {
    Parser::new(Input::Utf8(text)).parse()
}

/// One persistent value in a Java string; its cursor counts UTF-16 code units.
pub fn parse_snbt_utf16(text: &[u16]) -> Result<(Tag, usize), SnbtError> {
    Parser::new(Input::Utf16(text)).parse()
}

/// One value and its byte count, with a printable error for non-command callers.
pub fn parse_snbt(text: &str) -> Result<(Tag, usize), String> {
    parse_snbt_argument(text).map_err(|error| error.to_string())
}

/// `ParserBasedArgument.listSuggestions`: the grammar's longest-position suppliers.
pub fn snbt_suggestions(text: &str) -> (usize, Vec<String>) {
    let mut parser = Parser::new(Input::Utf8(text));
    parser.collect_suggestions = true;
    match parser.value() {
        Ok(value) => {
            super::discard_tag(value);
            // `ParseState.parseTopRule` discards errors strictly before a successful read.
            parser.record(parser.cursor, std::iter::empty());
        }
        Err(Failure {
            cursor,
            reason: Reason::DepthLimit,
        }) => return (cursor, Vec::new()),
        Err(error) => parser.defer(error),
    }
    (parser.suggestion_cursor, parser.suggestions)
}

/// `StringReader.skipWhitespace`, not Unicode's broader White_Space property.
pub fn snbt_whitespace(value: u32) -> bool {
    matches!(value, 0x09..=0x0d | 0x1c..=0x20 | 0x1680 | 0x2000..=0x2006
        | 0x2008..=0x200a | 0x2028..=0x2029 | 0x205f | 0x3000)
}

// Vanilla's SNBT grammar has no explicit depth limit, but its NBT reader uses
// NbtAccounter.MAX_STACK_DEPTH. Apply that bound here to protect tag consumers.
const MAX_PARSE_DEPTH: usize = 512;

struct Failure {
    cursor: usize,
    reason: Reason,
}

enum Reason {
    /// A resource refusal, not a recoverable grammar failure.
    DepthLimit,
    /// The ASCII characters accepted by one `StringReaderTerms` terminal.
    Literal(&'static str),
    Key {
        key: &'static str,
        arg: Option<String>,
        suggestions: &'static [&'static str],
    },
    IntegerRange {
        literal: IntegerLiteral,
        kind: u8,
        magnitude: Option<u64>,
    },
}

impl Reason {
    fn suggestions(&self) -> impl Iterator<Item = &str> {
        let (literal, words) = match self {
            Self::Literal(chars) => (*chars, &[][..]),
            Self::Key { suggestions, .. } => ("", *suggestions),
            Self::DepthLimit | Self::IntegerRange { .. } => ("", &[][..]),
        };
        (0..literal.len())
            .map(move |i| &literal[i..i + 1])
            .chain(words.iter().copied())
    }
}

struct IntegerLiteral {
    digits: Range<usize>,
    radix: u32,
    minus: bool,
    signed: bool,
    kind: Option<u8>,
}

struct Parser<'a> {
    input: Input<'a>,
    cursor: usize,
    depth: usize,
    collect_suggestions: bool,
    suggestion_cursor: usize,
    suggestions: Vec<String>,
    /// `ErrorCollector.LongestOnly` and `Grammar.parseForCommands` keep the
    /// first syntax exception at the furthest position, even after backtracking.
    deferred: Option<Failure>,
}
impl<'a> Parser<'a> {
    fn new(input: Input<'a>) -> Self {
        Self {
            input,
            cursor: 0,
            depth: 0,
            collect_suggestions: false,
            suggestion_cursor: 0,
            suggestions: Vec::new(),
            deferred: None,
        }
    }
    fn parse(mut self) -> Result<(Tag, usize), SnbtError> {
        match self.value() {
            Ok(value) => Ok((value, self.cursor)),
            Err(error) => {
                let Failure { cursor, reason } = if matches!(error.reason, Reason::DepthLimit) {
                    error
                } else {
                    match self.deferred.take() {
                        Some(kept) if kept.cursor >= error.cursor => kept,
                        _ => error,
                    }
                };
                let suggestions: Vec<_> = reason.suggestions().map(str::to_owned).collect();
                let (key, arg) = match reason {
                    Reason::DepthLimit => ("arguments.nbtpath.too_deep", None),
                    Reason::Literal(_) => {
                        ("argument.literal.incorrect", Some(suggestions.join("|")))
                    }
                    Reason::Key { key, arg, .. } => (key, arg),
                    Reason::IntegerRange {
                        literal,
                        kind,
                        magnitude,
                    } => (
                        "snbt.parser.number_parse_failure",
                        Some(self.integer_range_message(literal, kind, magnitude)),
                    ),
                };
                Err(SnbtError {
                    key,
                    args: arg.into_iter().collect(),
                    cursor,
                    suggestions,
                })
            }
        }
    }
    fn defer(&mut self, error: Failure) {
        if self.collect_suggestions {
            self.record(error.cursor, error.reason.suggestions());
        }
        if self
            .deferred
            .as_ref()
            .is_none_or(|kept| error.cursor > kept.cursor)
        {
            self.deferred = Some(error);
        }
    }
    fn error(&self, key: &'static str) -> Failure {
        self.diagnostic(key, None, &[])
    }
    fn diagnostic(
        &self,
        key: &'static str,
        arg: Option<String>,
        suggestions: &'static [&'static str],
    ) -> Failure {
        Failure {
            cursor: self.cursor,
            reason: Reason::Key {
                key,
                arg,
                suggestions,
            },
        }
    }
    fn integer_range_message(
        &self,
        literal: IntegerLiteral,
        kind: u8,
        magnitude: Option<u64>,
    ) -> String {
        // Only the selected diagnostic needs the text Java's number parser reports.
        let IntegerLiteral {
            digits,
            radix,
            minus,
            signed,
            ..
        } = literal;
        let mut text = String::new();
        if minus {
            text.push('-');
        }
        text.extend(self.input.clean_digits(digits).map(char::from));
        let for_input = || {
            if radix == 10 {
                format!("For input string: \"{text}\"")
            } else {
                format!("For input string: \"{text}\" under radix {radix}")
            }
        };
        match magnitude {
            Some(value) if kind < 2 && value <= i32::MAX as u64 + u64::from(minus) => {
                if signed {
                    format!("Value out of range. Value:\"{text}\" Radix:{radix}")
                } else {
                    format!("out of range: {value}")
                }
            }
            _ if !signed && kind == 2 => {
                format!("String value {text} exceeds range of unsigned int.")
            }
            None if !signed && kind == 3 => {
                format!("String value {text} exceeds range of unsigned long.")
            }
            _ => for_input(),
        }
    }
    fn record<'v>(&mut self, cursor: usize, values: impl Iterator<Item = &'v str>) {
        if cursor < self.suggestion_cursor {
            return;
        }
        if cursor > self.suggestion_cursor {
            self.suggestion_cursor = cursor;
            self.suggestions.clear();
        }
        for value in values {
            if !self.suggestions.iter().any(|present| present == value) {
                self.suggestions.push(value.to_owned());
            }
        }
    }
    fn skip(&mut self) {
        while self.peek().is_some_and(snbt_whitespace) {
            self.bump();
        }
    }
    fn peek(&self) -> Option<u32> {
        self.input.at(self.cursor)
    }
    fn bump(&mut self) -> Option<u32> {
        let value = self.peek()?;
        self.cursor += self.input.width(value);
        Some(value)
    }
    /// `Term.optional`: rewind a failed term without discarding its diagnostics.
    fn optional<T>(&mut self, parse: impl FnOnce(&mut Self) -> Result<T, Failure>) -> Option<T> {
        let mark = self.cursor;
        match parse(self) {
            Ok(value) => Some(value),
            Err(error) => {
                self.defer(error);
                self.cursor = mark;
                None
            }
        }
    }
    fn eat(&mut self, chars: &'static str) -> bool {
        self.optional(|parser| parser.expect(chars)).is_some()
    }
    /// `StringReaderTerms.TerminalCharacters.parse`: ignore leading whitespace,
    /// then report a mismatch at the first non-whitespace position.
    fn expect(&mut self, chars: &'static str) -> Result<(), Failure> {
        self.skip();
        if self
            .peek()
            .is_some_and(|c| chars.bytes().any(|value| c == u32::from(value)))
        {
            self.bump();
            Ok(())
        } else {
            Err(Failure {
                cursor: self.cursor,
                reason: Reason::Literal(chars),
            })
        }
    }
    /// `Term.repeatedWithTrailingSeparator`: try an entry before the closer,
    /// then a comma before each next entry. Only grammar failures backtrack.
    fn entries(
        &mut self,
        mut entry: impl FnMut(&mut Self) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        loop {
            let mark = self.cursor;
            match entry(self) {
                Ok(()) => {
                    if !self.eat(",") {
                        return Ok(());
                    }
                }
                Err(error) if matches!(error.reason, Reason::DepthLimit) => return Err(error),
                Err(error) => {
                    self.defer(error);
                    self.cursor = mark;
                    return Ok(());
                }
            }
        }
    }
    fn descend<T>(
        &mut self,
        parse: impl FnOnce(&mut Self) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(Failure {
                cursor: self.cursor,
                reason: Reason::DepthLimit,
            });
        }
        self.depth += 1;
        let result = super::with_tree_stack(|| parse(self));
        self.depth -= 1;
        result
    }
    /// The SNBT `literal` rule: structural starters commit via silent lookahead.
    /// A failed numeric rule falls back to a word, but a matched integer literal
    /// is converted after the alternative commits.
    fn value(&mut self) -> Result<Tag, Failure> {
        self.skip();
        match self.peek() {
            Some(0x7b) => self.descend(Self::compound),
            Some(0x5b) => self.descend(Self::list),
            Some(0x22 | 0x27) => self.quoted_string().map(Tag::String),
            Some(0x2b | 0x2d | 0x2e | 0x30..=0x39) => {
                if let Some(value) = self.optional(Self::float) {
                    Ok(value)
                } else if let Some(integer) = self.optional(Self::integer) {
                    self.convert_integer(integer, None)
                } else {
                    self.word()
                }
            }
            _ => self.word(),
        }
    }
    fn unquoted(&mut self) -> Result<String, Failure> {
        self.skip();
        let mut value = String::new();
        while let Some(c) = self.peek().and_then(|c| u8::try_from(c).ok()) {
            if !c.is_ascii_alphanumeric() && !matches!(c, b'_' | b'-' | b'.' | b'+') {
                break;
            }
            self.bump();
            value.push(char::from(c));
        }
        if value.is_empty() {
            Err(self.error("snbt.parser.expected_unquoted_string"))
        } else {
            Ok(value)
        }
    }
    fn quoted_string(&mut self) -> Result<JavaString, Failure> {
        let quote = if self.eat("\"") {
            0x22
        } else {
            self.expect("'")?;
            0x27
        };
        let mut units = Vec::new();
        loop {
            match self.bump() {
                Some(c) if c == quote => return Ok(JavaString(units)),
                Some(0x5c) => self.escape(&mut units)?,
                Some(c) => push_code_point(&mut units, c),
                None => {
                    // The contents rule fails first at the end; the closing quote, the other
                    // quote and a backslash are what could continue the string.
                    return Err(self.diagnostic(
                        "snbt.parser.invalid_string_contents",
                        None,
                        &["\"", "'", "\\"],
                    ));
                }
            }
        }
    }
    fn escape(&mut self, units: &mut Vec<u16>) -> Result<(), Failure> {
        // StringReaderTerms terminals skip whitespace even after a backslash;
        // fixed-width hex runs and ordinary quoted contents do not.
        self.skip();
        if self.collect_suggestions {
            self.record(
                self.cursor,
                [
                    "b", "s", "t", "n", "f", "r", "\\", "'", "\"", "x", "u", "U", "N",
                ]
                .into_iter(),
            );
        }
        let escape_cursor = self.cursor;
        let value = match self.bump() {
            Some(0x62) => 8,
            Some(0x73) => 32,
            Some(0x74) => 9,
            Some(0x6e) => 10,
            Some(0x66) => 12,
            Some(0x72) => 13,
            Some(c @ (0x5c | 0x27 | 0x22)) => c,
            Some(c @ (0x78 | 0x75 | 0x55)) => {
                let count = match c {
                    0x78 => 2,
                    0x75 => 4,
                    _ => 8,
                };
                let digits_cursor = self.cursor;
                let mut value = 0u32;
                for _ in 0..count {
                    let Some(digit) = self
                        .peek()
                        .and_then(char::from_u32)
                        .and_then(|c| c.to_digit(16))
                    else {
                        self.cursor = digits_cursor;
                        return Err(self.diagnostic(
                            "snbt.parser.expected_hex_escape",
                            Some(count.to_string()),
                            &[],
                        ));
                    };
                    self.bump();
                    value = (value << 4) | digit;
                }
                if value > 0x10ffff {
                    return Err(self.diagnostic(
                        "snbt.parser.invalid_codepoint",
                        Some(format!("U+{value:08X}")),
                        &[],
                    ));
                }
                value
            }
            Some(0x4e) => {
                self.expect("{")?;
                let mut name = String::new();
                while let Some(c) = self.peek().and_then(|c| u8::try_from(c).ok()) {
                    if !c.is_ascii_alphanumeric() && !matches!(c, b'-' | b' ') {
                        break;
                    }
                    self.bump();
                    name.push(char::from(c));
                }
                // `GreedyPatternParseRule` needs at least one name character and offers
                // no completion when it fails, so `\N{` suggests nothing, not `}`.
                if name.is_empty() {
                    return Err(self.error("snbt.parser.invalid_character_name"));
                }
                self.expect("}")?;
                crate::character_names::code_point_of(&name)
                    .ok_or_else(|| self.error("snbt.parser.invalid_character_name"))?
            }
            _ => {
                // The first escape alternative is what a failed literal reports.
                return Err(Failure {
                    cursor: escape_cursor,
                    reason: Reason::Literal("b"),
                });
            }
        };
        push_code_point(units, value);
        Ok(())
    }
    fn compound(&mut self) -> Result<Tag, Failure> {
        self.expect("{")?;
        let mut values = Compound::new();
        let entries = self.entries(|parser| {
            let key = match parser.optional(Self::quoted_string) {
                Some(key) => key,
                None => parser.unquoted()?.as_str().into(),
            };
            parser.expect(":")?;
            let value = parser.value()?;
            if key.0.is_empty() {
                super::discard_tag(value);
                return Err(parser.error("snbt.parser.empty_key"));
            }
            if let Some(previous) = values.insert(key, value) {
                super::discard_tag(previous);
            }
            Ok(())
        });
        let tag = Tag::Compound(values);
        if let Err(error) = entries.and_then(|()| self.expect("}")) {
            super::discard_tag(tag);
            return Err(error);
        }
        Ok(tag)
    }
    fn list(&mut self) -> Result<Tag, Failure> {
        self.expect("[")?;
        // Only the prefix and semicolon choose the typed branch. Entries may
        // be empty in either branch; a later failure does not change the choice.
        let typed = self.optional(|parser| {
            let kind = if parser.eat("B") {
                0
            } else if parser.eat("L") {
                3
            } else {
                parser.expect("I")?;
                2
            };
            parser.expect(";")?;
            Ok(kind)
        });
        if let Some(kind) = typed {
            let mut values = Vec::new();
            self.entries(|parser| {
                values.push(parser.integer()?);
                Ok(())
            })?;
            self.expect("]")?;
            let values = values
                .into_iter()
                .map(|value| self.convert_integer(value, Some(kind)).map(integer_value));
            Ok(match kind {
                0 => Tag::ByteArray(
                    values
                        .map(|value| value.map(|v| v as u8))
                        .collect::<Result<_, _>>()?,
                ),
                2 => Tag::IntArray(
                    values
                        .map(|value| value.map(|v| v as i32))
                        .collect::<Result<_, _>>()?,
                ),
                _ => Tag::LongArray(values.collect::<Result<_, _>>()?),
            })
        } else {
            let mut values = Vec::new();
            let entries = self.entries(|parser| {
                values.push(parser.value()?);
                Ok(())
            });
            let tag = Tag::List(values);
            if let Err(error) = entries.and_then(|()| self.expect("]")) {
                super::discard_tag(tag);
                return Err(error);
            }
            Ok(tag)
        }
    }
    fn sign(&mut self) -> bool {
        !self.eat("+") && self.eat("-")
    }
    /// `NumberRunParseRule.parse`: a rejected run is reported at its start.
    fn digits(&mut self, radix: u32) -> Result<Range<usize>, Failure> {
        self.skip();
        let start = self.cursor;
        let mut last = None;
        while let Some(c) = self.peek().and_then(char::from_u32) {
            if c != '_' && !c.is_digit(radix) {
                break;
            }
            self.bump();
            last = Some(c);
        }
        if last == Some('_') || self.input.at(start) == Some(u32::from(b'_')) {
            self.cursor = start;
            return Err(self.error("snbt.parser.underscore_not_allowed"));
        }
        if last.is_none() {
            Err(self.error(match radix {
                2 => "snbt.parser.expected_binary_numeral",
                16 => "snbt.parser.expected_hex_numeral",
                _ => "snbt.parser.expected_decimal_numeral",
            }))
        } else {
            Ok(start..self.cursor)
        }
    }
    fn exponent(&mut self) -> Result<(bool, Range<usize>), Failure> {
        self.expect("eE")?;
        let minus = self.sign();
        Ok((minus, self.digits(10)?))
    }
    fn float(&mut self) -> Result<Tag, Failure> {
        let minus = self.sign();
        let whole = self.optional(|parser| parser.digits(10));
        // The first two float readings commit when they consume the dot. A
        // leading dot needs a fraction; a dot after digits does not.
        let (dot, fraction) = if whole.is_none() {
            self.expect(".")?;
            (true, Some(self.digits(10)?))
        } else if self.eat(".") {
            (true, self.optional(|parser| parser.digits(10)))
        } else {
            (false, None)
        };
        let exponent = self.optional(Self::exponent);
        let is_float = self.eat("fF");
        if !is_float {
            if dot || exponent.is_some() {
                self.eat("dD");
            } else {
                self.expect("dD")?;
            }
        }

        let mut text = String::new();
        if minus {
            text.push('-');
        }
        if let Some(whole) = whole {
            text.extend(self.input.clean_digits(whole).map(char::from));
        }
        if dot {
            text.push('.');
        }
        if let Some(fraction) = fraction {
            text.extend(self.input.clean_digits(fraction).map(char::from));
        }
        if let Some((minus, digits)) = exponent {
            text.push('e');
            if minus {
                text.push('-');
            }
            text.extend(self.input.clean_digits(digits).map(char::from));
        }
        if is_float {
            let value = text
                .parse::<f32>()
                .map_err(|_| self.error("snbt.parser.number_parse_failure"))?;
            if !value.is_finite() {
                return Err(self.error("snbt.parser.infinity_not_allowed"));
            }
            Ok(Tag::Float(if value == 0.0 { 0 } else { value.to_bits() }))
        } else {
            let value = text
                .parse::<f64>()
                .map_err(|_| self.error("snbt.parser.number_parse_failure"))?;
            if !value.is_finite() {
                return Err(self.error("snbt.parser.infinity_not_allowed"));
            }
            Ok(Tag::Double(if value == 0.0 { 0 } else { value.to_bits() }))
        }
    }
    /// `integer_literal` is syntactic: scalar conversion belongs to `literal`,
    /// and array conversion to `list_literal` after the closing bracket.
    fn integer(&mut self) -> Result<IntegerLiteral, Failure> {
        let minus = self.sign();
        let (digits, radix) = if self.eat("0") {
            let zero = self.cursor - 1..self.cursor;
            if self.eat("xX") {
                (self.digits(16)?, 16)
            } else if let Some(binary) = self.optional(|parser| {
                parser.expect("bB")?;
                parser.digits(2)
            }) {
                (binary, 2)
            } else if self.optional(|parser| parser.digits(10)).is_some() {
                return Err(self.error("snbt.parser.leading_zero_not_allowed"));
            } else {
                (zero, 10)
            }
        } else {
            (self.digits(10)?, 10)
        };
        let (kind, signed) = if let Some(kind) = self.optional(|parser| {
            parser.expect("uU")?;
            parser.integer_type()
        }) {
            (Some(kind), false)
        } else if let Some(kind) = self.optional(|parser| {
            parser.expect("sS")?;
            parser.integer_type()
        }) {
            (Some(kind), true)
        } else {
            (self.optional(Self::integer_type), radix == 10)
        };
        Ok(IntegerLiteral {
            digits,
            radix,
            minus,
            signed,
            kind,
        })
    }
    fn integer_type(&mut self) -> Result<u8, Failure> {
        if self.eat("bB") {
            Ok(0)
        } else if self.eat("sS") {
            Ok(1)
        } else if self.eat("iI") {
            Ok(2)
        } else {
            self.expect("lL")?;
            Ok(3)
        }
    }
    fn convert_integer(&self, literal: IntegerLiteral, array: Option<u8>) -> Result<Tag, Failure> {
        let IntegerLiteral {
            radix,
            minus,
            signed,
            kind,
            ..
        } = literal;
        let kind = kind.unwrap_or(array.unwrap_or(2));
        if array.is_some_and(|array| kind > array) {
            return Err(self.error("snbt.parser.invalid_array_element_type"));
        }
        if minus && !signed {
            return Err(self.error("snbt.parser.expected_non_negative_number"));
        }
        let magnitude =
            self.input
                .clean_digits(literal.digits.clone())
                .try_fold(0u64, |value, byte| {
                    let digit = char::from(byte).to_digit(radix).expect("scanned digit");
                    value
                        .checked_mul(u64::from(radix))?
                        .checked_add(u64::from(digit))
                });
        let bits = [8, 16, 32, 64][usize::from(kind)];
        let max = if signed {
            (1u64 << (bits - 1)) - 1 + u64::from(minus)
        } else if bits == 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        let Some(magnitude_in_range) = magnitude.filter(|&value| value <= max) else {
            return Err(Failure {
                cursor: self.cursor,
                reason: Reason::IntegerRange {
                    literal,
                    kind,
                    magnitude,
                },
            });
        };
        let value = if minus {
            magnitude_in_range.wrapping_neg() as i64
        } else {
            magnitude_in_range as i64
        };
        Ok(match kind {
            0 => Tag::Byte(value as i8),
            1 => Tag::Short(value as i16),
            2 => Tag::Int(value as i32),
            _ => Tag::Long(value),
        })
    }
    fn word(&mut self) -> Result<Tag, Failure> {
        let word = self.unquoted()?;
        let mark = self.cursor;
        let arguments = if self.eat("(") {
            Some(self.descend(|parser| {
                let mut arguments = Vec::new();
                if let Err(error) = parser.entries(|parser| {
                    arguments.push(parser.value()?);
                    Ok(())
                }) {
                    super::discard_tag(Tag::List(arguments));
                    return Err(error);
                }
                if let Err(error) = parser.expect(")") {
                    // The optional sequence rewinds the reader, but the named
                    // argument list is already in the enclosing rule's scope.
                    parser.defer(error);
                    parser.cursor = mark;
                }
                Ok(arguments)
            })?)
        } else {
            None
        };
        if matches!(word.as_bytes()[0], b'+' | b'-' | b'.' | b'0'..=b'9') {
            if let Some(arguments) = arguments {
                super::discard_tag(Tag::List(arguments));
            }
            return Err(self.diagnostic(
                "snbt.parser.invalid_unquoted_start",
                None,
                &["false", "true", "bool", "uuid"],
            ));
        }
        if let Some(arguments) = arguments {
            return self.operation(&word, arguments);
        }
        if word.eq_ignore_ascii_case("true") {
            Ok(Tag::Byte(1))
        } else if word.eq_ignore_ascii_case("false") {
            Ok(Tag::Byte(0))
        } else {
            Ok(Tag::String(word.as_str().into()))
        }
    }
    fn operation(&self, word: &str, arguments: Vec<Tag>) -> Result<Tag, Failure> {
        let result = (|| match (word, arguments.as_slice()) {
            ("bool", [value]) => {
                let value = match value {
                    Tag::Byte(v) => f64::from(*v),
                    Tag::Short(v) => f64::from(*v),
                    Tag::Int(v) => f64::from(*v),
                    Tag::Long(v) => *v as f64,
                    Tag::Float(v) => f64::from(f32::from_bits(*v)),
                    Tag::Double(v) => f64::from_bits(*v),
                    _ => return Err(self.error("snbt.parser.expected_number_or_boolean")),
                };
                Ok(Tag::Byte(i8::from(value != 0.0)))
            }
            ("uuid", [Tag::String(value)]) => {
                let value = value
                    .to_string()
                    .ok()
                    .and_then(|value| crate::text::parse_java_uuid(&value))
                    .ok_or_else(|| self.error("snbt.parser.expected_string_uuid"))?;
                Ok(Tag::IntArray(vec![
                    (value >> 96) as u32 as i32,
                    (value >> 64) as u32 as i32,
                    (value >> 32) as u32 as i32,
                    value as u32 as i32,
                ]))
            }
            ("uuid", [_]) => Err(self.error("snbt.parser.expected_string_uuid")),
            _ => Err(self.diagnostic(
                "snbt.parser.no_such_operation",
                Some(format!("{word}/{}", arguments.len())),
                &[],
            )),
        })();
        super::discard_tag(Tag::List(arguments));
        result
    }
}
fn integer_value(value: Tag) -> i64 {
    match value {
        Tag::Byte(value) => i64::from(value),
        Tag::Short(value) => i64::from(value),
        Tag::Int(value) => i64::from(value),
        Tag::Long(value) => value,
        _ => unreachable!("integer array elements are validated before construction"),
    }
}
fn push_code_point(units: &mut Vec<u16>, value: u32) {
    if value <= 0xffff {
        units.push(value as u16);
    } else {
        let value = value - 0x10000;
        units.push(0xd800 | (value >> 10) as u16);
        units.push(0xdc00 | (value & 0x3ff) as u16);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_persistent_grammar_preserves_utf16_and_numeric_types() {
        let text = r#"{byte:0xffub,short:65535us,int:0xffffffff,long:0xffffffffffffffffL,signed:-0x80sb,binary:0b1111,float:1 . 25 f,array:[I;1b,2s,3],flags:[TRUE,FALSE,bool(256)],uuid:uuid('1-1-1-1-1'),text:'\x41\uD800\U0001F600\N{snowman}'}"#;
        let (tag, used) = parse_snbt_argument(text).unwrap();
        assert_eq!(used, text.len());
        assert_eq!(tag.get("byte"), Some(&Tag::Byte(-1)));
        assert_eq!(tag.get("short"), Some(&Tag::Short(-1)));
        assert_eq!(tag.get("int"), Some(&Tag::Int(-1)));
        assert_eq!(tag.get("long"), Some(&Tag::Long(-1)));
        assert_eq!(tag.get("signed"), Some(&Tag::Byte(-128)));
        assert_eq!(tag.get("binary"), Some(&Tag::Int(15)));
        assert_eq!(tag.get("float"), Some(&Tag::Float(1.25f32.to_bits())));
        assert_eq!(tag.get("array"), Some(&Tag::IntArray(vec![1, 2, 3])));
        assert_eq!(
            tag.get("flags"),
            Some(&Tag::List(vec![Tag::Byte(1), Tag::Byte(0), Tag::Byte(1)]))
        );
        assert_eq!(
            tag.get("uuid"),
            Some(&Tag::IntArray(vec![1, 65537, 65536, 1]))
        );
        assert_eq!(
            tag.get("text"),
            Some(&Tag::String(JavaString(vec![
                0x41, 0xd800, 0xd83d, 0xde00, 0x2603
            ]))),
        );
        let units: Vec<u16> = "{'"
            .encode_utf16()
            .chain([0xd800])
            .chain("':'".encode_utf16())
            .chain([0xdc00])
            .chain("'}".encode_utf16())
            .collect();
        assert_eq!(
            parse_snbt_utf16(&units).unwrap(),
            (
                Tag::compound([(
                    JavaString(vec![0xd800]),
                    Tag::String(JavaString(vec![0xdc00]))
                )]),
                units.len()
            ),
        );
        for source in [
            "{v:128b}",
            "{v:65536us}",
            "{v:0xffsb}",
            "{v:-0xff}",
            "{v:1e400}",
            "{v:01}",
            "{v:[B;1s]}",
            "{v:[I;1L]}",
            "{v:'\\U00110000'}",
            "{v:'\\u123'}",
            "{v:'\\q'}",
            "{v:'\\N{LOW-LINE}'}",
            "{v:bool('true')}",
            "{v:uuid('invalid')}",
            "{v:unknown(1)}",
            "{'':1}",
        ] {
            assert!(parse_snbt_argument(source).is_err(), "{source}");
        }
        assert_eq!(
            crate::text::parse_java_uuid("١-１-1-1-1"),
            Some(0x00000001_00010001_00010000_00000001)
        );
    }

    /// `GreedyPatternParseRule` for the name fails without suggestions, so an
    /// unfinished `\N{` offers nothing; the closing brace is only expected after a name.
    #[test]
    fn unfinished_unicode_name_escape_suggestions() {
        assert_eq!(snbt_suggestions("\"\\N{"), (4, Vec::new()));
        assert_eq!(snbt_suggestions("\"\\N{LATIN"), (9, vec!["}".to_owned()]));
        let finished = "\"\\N{LATIN SMALL LETTER A}\"";
        assert_eq!(snbt_suggestions(finished), (finished.len(), Vec::new()));
        assert_eq!(
            parse_snbt_argument("'\\N{!}'").unwrap_err().key,
            "snbt.parser.invalid_character_name"
        );
    }

    #[test]
    fn failed_alternatives_report_the_first_terminal_at_the_furthest_cursor() {
        for (source, expected, cursor) in [
            ("{]", "\"", 1),
            ("{", "\"", 1),
            ("{,}", "\"", 1),
            ("{:1}", "\"", 1),
            ("[ x", "(", 3),
            ("[", "B", 1),
            ("{a:[}", "B", 4),
            ("{a:1", ".", 4),
            ("{a:1 x", ".", 5),
            ("{a:99999999999}", ".", 14),
            ("{\"\":1}", ".", 5),
            ("{a:1.2.3}", "e|E", 6),
            ("{a:--1}", "(", 6),
            ("{a:[I;1.5]}", "u|U", 7),
            ("[B;1", "u|U", 4),
            ("[B;1 x", "u|U", 5),
        ] {
            let error = parse_snbt_argument(source).unwrap_err();
            assert_eq!(error.key, "argument.literal.incorrect", "{source}");
            assert_eq!(error.args, [expected], "{source}");
            assert_eq!(error.cursor, cursor, "{source}");
        }
    }

    #[test]
    fn nesting_refusal_is_not_a_backtrackable_syntax_error() {
        for (open, close, cursor) in [("[", "]", 512), ("{a:", "}", 1536), ("bool(", ")", 2565)] {
            let source = format!("{}0{}", open.repeat(513), close.repeat(513));
            let error = parse_snbt_argument(&source).unwrap_err();
            assert_eq!(
                (error.key, error.cursor),
                ("arguments.nbtpath.too_deep", cursor)
            );
            let units: Vec<_> = source.encode_utf16().collect();
            assert_eq!(parse_snbt_utf16(&units), Err(error));
            assert_eq!(snbt_suggestions(&source), (cursor, Vec::new()));
        }
    }

    #[test]
    fn short_suffix_keeps_signed_suffix_completions_at_its_end() {
        assert_eq!(parse_snbt_argument("1s"), Ok((Tag::Short(1), 2)));
        let (cursor, mut suggestions) = snbt_suggestions("1s");
        suggestions.sort_unstable();
        assert_eq!(cursor, 2);
        assert_eq!(suggestions, ["B", "I", "L", "S", "b", "i", "l", "s"]);
    }

    #[test]
    fn an_unclosed_builtin_rewinds_but_keeps_its_arguments() {
        assert_eq!(parse_snbt_argument("bool(1"), Ok((Tag::Byte(1), 4)));
        let error = parse_snbt_argument("foo(").unwrap_err();
        assert_eq!(error.key, "snbt.parser.expected_unquoted_string");
        assert_eq!(error.cursor, 4);
    }

    #[test]
    fn typed_array_converts_radix_spans_after_the_closer() {
        let source = "[I;1_2i,0xFFui,0b1_1]";
        let expected = Tag::IntArray(vec![12, 255, 3]);
        assert_eq!(parse_snbt_argument(source), Ok((expected.clone(), 21)));
        let units: Vec<_> = source.encode_utf16().collect();
        assert_eq!(parse_snbt_utf16(&units), Ok((expected, 21)));
    }

    #[test]
    fn unsigned_int_overflow_reports_the_unsigned_range_error() {
        for (source, digits, cursor) in [
            ("9223372036854775808ui", "9223372036854775808", 21),
            ("18446744073709551616ui", "18446744073709551616", 22),
            ("[I;18446744073709551616ui]", "18446744073709551616", 26),
        ] {
            let error = parse_snbt_argument(source).unwrap_err();
            assert_eq!(error.key, "snbt.parser.number_parse_failure");
            assert_eq!(error.cursor, cursor);
            assert_eq!(
                error.args,
                [format!(
                    "String value {digits} exceeds range of unsigned int."
                )]
            );
        }
    }

    /// `FloatLiteral | IntegerLiteral`: a malformed or out-of-range float falls back to the
    /// integer reading, optional numeric terms rewind, and the furthest failed alternative
    /// is still what a later failure reports.
    #[test]
    fn failed_numeric_alternatives_rewind() {
        let double = |value: f64| Tag::Double(value.to_bits());
        assert_eq!(parse_snbt_argument("1e").unwrap(), (Tag::Int(1), 1));
        assert_eq!(parse_snbt_argument("1e309").unwrap(), (Tag::Int(1), 1));
        assert_eq!(parse_snbt_argument("1.5e").unwrap(), (double(1.5), 3));
        assert_eq!(parse_snbt_argument("1e5").unwrap(), (double(1e5), 3));
        assert_eq!(parse_snbt_argument("1._").unwrap(), (double(1.0), 2));
        assert_eq!(parse_snbt_argument("0b_").unwrap(), (Tag::Byte(0), 2));
        let error = parse_snbt_argument("[1e309]").unwrap_err();
        // The optional float suffix fails before semantic conversion detects infinity.
        assert_eq!((error.key, error.cursor), ("argument.literal.incorrect", 6));
        assert_eq!(error.args, ["f|F"]);
        assert!(parse_snbt_argument("{v:1e}").is_err());
    }
}
