//! Byte ranges of styled parts, keeping only a cursor and the style carried at a break.
use super::Font;
use crate::text::{Span, Style, Text, legacy_format};
use std::borrow::Cow;

#[derive(Clone, Copy)]
struct Position {
    part: usize,
    byte: usize,
}

/// A line from `Font::formatted_lines_with_font`, borrowing the original text's parts.
pub struct FormattedLine<'a> {
    parts: &'a [Span],
    start: Position,
    end: Position,
    carried: Option<Style>,
}

impl FormattedLine<'_> {
    /// Raw substrings and their starting styles. Legacy codes within each substring still apply.
    pub fn ranges(&self) -> impl Iterator<Item = (&Span, &str, &Style)> {
        let after_end = (self.end.part + 1).min(self.parts.len());
        self.parts[self.start.part..after_end]
            .iter()
            .enumerate()
            .filter_map(move |(i, part)| {
                let mut from = 0;
                let mut style = &part.style;
                if i == 0 {
                    from = self.start.byte;
                    style = self.carried.as_ref().unwrap_or(style);
                }
                let to = if self.start.part + i == self.end.part {
                    self.end.byte
                } else {
                    part.text.len()
                };
                if from < to {
                    Some((part, &part.text[from..to], style))
                } else {
                    None
                }
            })
    }
}

/// The `StringSplitter` line walk: retain the tail's position, not a copy of its characters.
pub struct FormattedLines<'a> {
    font: &'a Font,
    parts: &'a [Span],
    default_font: &'a str,
    max_width: f32,
    start: Position,
    carried: Option<Style>,
    trailing_newline: bool,
}

impl Font {
    /// Wrap the styled parts of a component, visiting borrowed substrings rather than
    /// building a list of shortened components. The last explicit newline leaves a blank line.
    pub fn formatted_lines_with_font<'a>(
        &'a self,
        text: &'a Text,
        max_width: f32,
        default_font: &'a str,
    ) -> FormattedLines<'a> {
        let parts = &text.spans;
        FormattedLines {
            font: self,
            parts,
            default_font,
            max_width: max_width.max(1.0),
            start: Position {
                part: first_part(parts, 0),
                byte: 0,
            },
            carried: None,
            trailing_newline: false,
        }
    }
}

fn first_part(parts: &[Span], start: usize) -> usize {
    parts[start..]
        .iter()
        .position(|part| !part.text.is_empty())
        .map_or(parts.len(), |offset| start + offset)
}

impl<'a> Iterator for FormattedLines<'a> {
    type Item = FormattedLine<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.start;
        if start.part == self.parts.len() && !std::mem::take(&mut self.trailing_newline) {
            return None;
        }
        let carried = self.carried.take();
        let end = {
            let mut width = 0.0;
            let mut had_visible = false;
            let mut last_space = None;
            let mut split = None;
            // StringSplitter.LineBreakFinder: the first visible glyph can overflow on its own;
            // afterwards prefer the last space, otherwise break immediately before the first overflow.
            'line: for (i, part) in self.parts.iter().enumerate().skip(start.part) {
                let (byte, initial) = if i == start.part {
                    (start.byte, carried.as_ref().unwrap_or(&part.style))
                } else {
                    (0, &part.style)
                };
                let mut style = Cow::Borrowed(initial);
                let mut contents = part.text[byte..].char_indices();
                while let Some((offset, c)) = contents.next() {
                    if c == '§' {
                        // A trailing formatting introducer ends only this part.
                        let Some((_, code)) = contents.next() else {
                            break;
                        };
                        style = Cow::Owned(legacy_format(&style, code, &Style::default()));
                        continue;
                    }
                    let pos = Position {
                        part: i,
                        byte: byte + offset,
                    };
                    if c == '\n' {
                        split = Some((pos, c, style));
                        break 'line;
                    }
                    if c == ' ' {
                        last_space = Some((pos, c, style.clone()));
                    }
                    let advance = self.font.advance_with_default(c, &style, self.default_font);
                    width += advance;
                    if had_visible && width > self.max_width {
                        split = last_space.or(Some((pos, c, style)));
                        break 'line;
                    }
                    had_visible |= advance != 0.0;
                }
            }
            if let Some((pos, c, style)) = split {
                self.trailing_newline = c == '\n';
                let byte = pos.byte + usize::from(c == ' ' || c == '\n');
                self.start = if byte == self.parts[pos.part].text.len() {
                    Position {
                        part: first_part(self.parts, pos.part + 1),
                        byte: 0,
                    }
                } else {
                    // If the breakpoint is at the start of an untouched part and no
                    // delimiter was consumed, it already has the right starting style.
                    self.carried = (byte != 0).then(|| style.into_owned());
                    Position {
                        part: pos.part,
                        byte,
                    }
                };
                pos
            } else {
                self.trailing_newline = false;
                self.start = Position {
                    part: self.parts.len(),
                    byte: 0,
                };
                self.start
            }
        };
        Some(FormattedLine {
            parts: self.parts,
            start,
            end,
            carried,
        })
    }
}
