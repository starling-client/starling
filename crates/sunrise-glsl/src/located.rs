//! Where a stage's code came from, carried through the rewrites so an error naga finds
//! names the file, line and byte column the author wrote ([`Error`]).

use std::ops::Range;

/// A diagnostic in the author's file (`ns:path`, as includes name it). Its display includes
/// the authored position and the shader stage when known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub file: String,
    /// Line from 1, or 0 for the file as a whole.
    pub line: u32,
    /// Byte column from 1 in the author's line, or 0 when no column is known.
    pub column: u32,
    /// The shader stage that raised the error, when it can be attributed to one stage.
    pub stage: Option<naga::ShaderStage>,
    /// Why translation failed, without a stage prefix.
    pub message: String,
    /// The block or sampler a pipeline does not provide, for the error that says so (a program
    /// reads a name its pipeline neither binds nor provides); `None` for every other error.
    pub unprovided: Option<String>,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.file.as_str(), self.line, self.column) {
            ("", _, _) => {}
            (file, 0, _) => write!(f, "{file}: ")?,
            (file, line, 0) => write!(f, "{file}:{line}: ")?,
            (file, line, column) => write!(f, "{file}:{line}:{column}: ")?,
        }
        std::fmt::Display::fmt(&shader_message(self.stage, &self.message), f)
    }
}

impl std::error::Error for Error {}

impl Error {
    /// `message` at `line` of `file` (line 0: the file as a whole), with no column or stage.
    pub fn new(file: impl Into<String>, line: u32, message: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            line,
            column: 0,
            stage: None,
            message: message.into(),
            unprovided: None,
        }
    }

    /// Attribute the error to `stage`, without changing the refusal message.
    pub fn with_stage(mut self, stage: naga::ShaderStage) -> Self {
        self.stage = Some(stage);
        self
    }

    /// This error, saying `name` is the block or sampler the pipeline does not provide.
    pub fn with_unprovided(mut self, name: impl Into<String>) -> Self {
        self.unprovided = Some(name.into());
        self
    }
}

/// Where something is in the author's files: a file id and a line from 1, as [`Error`] has them
/// (line 0: the file as a whole; no file: nowhere known, for a program made without its
/// sources).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Location {
    pub file: String,
    pub line: u32,
}

impl Location {
    /// `message` here.
    pub fn error(&self, message: impl std::fmt::Display) -> Error {
        Error::new(self.file.clone(), self.line, message.to_string())
    }
}

/// Display a refusal with its shader stage as a prefix when known, without changing the refusal
/// itself. Use this in diagnostics that carry their own source location.
pub fn shader_message(
    stage: Option<naga::ShaderStage>,
    message: impl std::fmt::Display,
) -> impl std::fmt::Display {
    use naga::ShaderStage;
    std::fmt::from_fn(move |f| {
        if let Some(stage) = stage {
            let name = match stage {
                ShaderStage::Vertex => "vertex",
                ShaderStage::Fragment => "fragment",
                ShaderStage::Compute => "compute",
                ShaderStage::Task => "task",
                ShaderStage::Mesh => "mesh",
                ShaderStage::RayGeneration => "ray generation",
                ShaderStage::Miss => "miss",
                ShaderStage::AnyHit => "any hit",
                ShaderStage::ClosestHit => "closest hit",
            };
            write!(f, "{name} shader: ")?;
        }
        message.fmt(f)
    })
}

/// Errors as one line, `; ` between them: a load note with each error's position and stage.
pub fn describe(errors: &[Error]) -> String {
    let mut out = String::new();
    for (n, error) in errors.iter().enumerate() {
        if n > 0 {
            out.push_str("; ");
        }
        out.push_str(&error.to_string());
    }
    out
}

/// A line's file (an index into [`Expanded`]'s files) and its line number from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Origin {
    pub file: u32,
    pub line: u32,
}

/// An authored byte position. A rewrite can copy a run of bytes or anchor a replacement to
/// one position, without keeping a source-map entry per byte.
#[derive(Clone, Copy)]
struct Position {
    origin: Origin,
    column: u32,
}

#[derive(Clone, Debug)]
struct Columns {
    range: Range<usize>,
    origin: Origin,
    column: u32,
    copied: bool,
}

impl Columns {
    fn column_at(&self, at: usize) -> u32 {
        let distance = if self.copied {
            at - self.range.start
        } else {
            0
        };
        self.column
            .saturating_add(u32::try_from(distance).unwrap_or(u32::MAX))
    }
}

/// Text and where its bytes came from. The ordered column runs map copies and replacements;
/// each line also has an origin for diagnostics that know only its line (`None` for new code).
#[derive(Clone, Debug)]
pub(crate) struct Lines {
    text: String,
    origins: Vec<Option<Origin>>,
    columns: Vec<Columns>,
}

impl Default for Lines {
    fn default() -> Self {
        Self {
            text: String::new(),
            origins: vec![None],
            columns: Vec::new(),
        }
    }
}

impl Lines {
    /// `text` with all its lines from `origin`.
    pub fn uniform(text: &str, origin: Option<Origin>) -> Self {
        let mut lines = Self::default();
        lines.push(text, origin);
        lines
    }

    /// `text` as file 0 wrote it, line for line.
    #[cfg(test)]
    pub fn from_text(text: &str) -> Self {
        let mut lines = Self::default();
        for (n, piece) in text.split_inclusive('\n').enumerate() {
            let line = u32::try_from(n + 1).unwrap_or(u32::MAX);
            lines.push_authored(piece, Origin { file: 0, line });
        }
        lines
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Keep the line origins of the input to [`crate::run_directives`]. It keeps all the line
    /// breaks but may leave a line empty or fill it with mapped text from a later line.
    pub fn with_line_origins(mut self, input: &Self) -> Self {
        debug_assert_eq!(self.origins.len(), input.origins.len());
        self.origins.clone_from(&input.origins);
        self
    }

    /// The line (from 0) holding byte `at`.
    pub fn line_of(&self, at: usize) -> usize {
        self.text.as_bytes()[..at.min(self.text.len())]
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count()
    }

    /// Where line `line` (from 0) starts.
    pub fn line_start(&self, line: usize) -> usize {
        match line.checked_sub(1) {
            None => 0,
            Some(before) => self
                .text
                .match_indices('\n')
                .nth(before)
                .map_or(self.text.len(), |(at, _)| at + 1),
        }
    }

    /// Where the line holding byte `at` came from.
    pub fn origin_at(&self, at: usize) -> Option<Origin> {
        self.origin(self.line_of(at))
    }

    /// Where line `line` (from 0) came from.
    pub fn origin(&self, line: usize) -> Option<Origin> {
        self.origins.get(line).copied().flatten()
    }

    /// The origins of the line holding byte `at` and of every line after it.
    pub fn origins_from(&self, at: usize) -> &[Option<Origin>] {
        &self.origins[self.line_of(at)..]
    }

    /// Appends `text`, its lines from `origin`. A line it continues keeps the origin it has,
    /// unless nothing is on it yet.
    pub fn push(&mut self, text: &str, origin: Option<Origin>) {
        if text.is_empty() {
            return;
        }
        if self.at_line_start() {
            *self.origins.last_mut().expect("one origin per line") = origin;
        }
        for _ in text.matches('\n') {
            self.origins.push(origin);
        }
        self.text.push_str(text);
    }

    /// Append one physical line (or its prefix before a continuation), starting at column 1.
    pub fn push_authored(&mut self, text: &str, origin: Origin) {
        let from = self.text.len();
        self.push(text, Some(origin));
        self.map(from..self.text.len(), Position { origin, column: 1 }, true);
    }

    /// Append `text` with the byte positions of an equally long `range` of `src`. For copies
    /// and for changes that keep every byte offset (comment blanking).
    pub fn push_spanned(
        &mut self,
        text: &str,
        origin: Option<Origin>,
        src: &Lines,
        range: Range<usize>,
    ) {
        debug_assert_eq!(text.len(), range.len());
        let from = self.text.len();
        self.push(text, origin);
        self.copy_columns(src, range, from);
    }

    /// Append replacement text, all of it anchored at `at` in `src` if that byte is authored.
    pub fn push_replacement(&mut self, text: &str, origin: Option<Origin>, src: &Lines, at: usize) {
        let from = self.text.len();
        self.push(text, origin);
        if let Some(pos) = src.position(at) {
            self.map(from..self.text.len(), pos, false);
        }
    }

    fn map(&mut self, range: Range<usize>, pos: Position, copied: bool) {
        if range.is_empty() {
            return;
        }
        if let Some(last) = self.columns.last_mut()
            && last.range.end == range.start
            && last.origin == pos.origin
            && last.copied == copied
            && last.column_at(range.start) == pos.column
        {
            last.range.end = range.end;
        } else {
            self.columns.push(Columns {
                range,
                origin: pos.origin,
                column: pos.column,
                copied,
            });
        }
    }

    fn copy_columns(&mut self, src: &Lines, range: Range<usize>, to: usize) {
        let first = src
            .columns
            .partition_point(|run| run.range.end <= range.start);
        for run in &src.columns[first..] {
            if run.range.start >= range.end {
                break;
            }
            let start = run.range.start.max(range.start);
            let end = run.range.end.min(range.end);
            self.map(
                to + (start - range.start)..to + (end - range.start),
                Position {
                    origin: run.origin,
                    column: run.column_at(start),
                },
                run.copied,
            );
        }
    }

    fn position(&self, at: usize) -> Option<Position> {
        let n = self.columns.partition_point(|run| run.range.end <= at);
        let run = self.columns.get(n).filter(|run| run.range.start <= at)?;
        Some(Position {
            origin: run.origin,
            column: run.column_at(at),
        })
    }

    /// Appends `other` with its origins, under the same rule as [`Lines::push`].
    pub fn push_lines(&mut self, other: &Lines) {
        if other.text.is_empty() {
            return;
        }
        if self.at_line_start() {
            *self.origins.last_mut().expect("one origin per line") = other.origins[0];
        }
        self.origins.extend_from_slice(&other.origins[1..]);
        self.copy_columns(other, 0..other.text.len(), self.text.len());
        self.text.push_str(&other.text);
    }

    fn at_line_start(&self) -> bool {
        self.text.is_empty() || self.text.ends_with('\n')
    }

    /// Appends `src`'s text in `range`, whose first byte is on `src`'s line `line`, with the
    /// origins of its lines and its columns; returns the line of `range.end`.
    fn push_slice(&mut self, src: &Lines, range: Range<usize>, line: usize) -> usize {
        self.copy_columns(src, range.clone(), self.text.len());
        let mut line = line;
        for piece in src.text[range].split_inclusive('\n') {
            self.push(piece, src.origins[line]);
            if piece.ends_with('\n') {
                line += 1;
            }
        }
        line
    }

    /// A subrange with its line and byte positions.
    pub fn slice(&self, range: Range<usize>) -> Self {
        let mut out = Self::default();
        out.push_slice(self, range.clone(), self.line_of(range.start));
        out
    }

    /// Each range (in order, apart) replaced by its text. A replacement with as many line breaks
    /// as the text it replaces is anchored to the start of each replaced piece, line for line.
    /// Any other replacement is anchored to the start of its range.
    pub fn edit(&mut self, edits: Vec<(Range<usize>, String)>) {
        if edits.is_empty() {
            return;
        }
        let mut line = 0;
        let mut pos = 0;
        let edits = edits
            .into_iter()
            .map(|(range, with)| {
                line += self.text[pos..range.start].matches('\n').count();
                pos = range.start;
                let replaced = self.text[range.clone()].matches('\n').count();
                let mut lines = Lines::default();
                if with.matches('\n').count() == replaced {
                    let mut at = range.start;
                    for (n, piece) in with.split_inclusive('\n').enumerate() {
                        lines.push_replacement(piece, self.origins[line + n], self, at);
                        at += self.text[at..range.end]
                            .find('\n')
                            .map_or(range.end - at, |i| i + 1);
                    }
                } else {
                    lines.push_replacement(&with, self.origins[line], self, range.start);
                }
                (range, lines)
            })
            .collect();
        self.edit_lines(edits);
    }

    /// Each range (in order, apart) replaced by its lines, which bring their own origins. The
    /// first line of a replacement continues the line before its range when that has text; the
    /// text after the range continues its last line, taking its own origin when that is empty.
    pub fn edit_lines(&mut self, edits: Vec<(Range<usize>, Lines)>) {
        let mut out = Lines::default();
        let (mut pos, mut line) = (0, 0);
        for (range, with) in edits {
            line = out.push_slice(self, pos..range.start, line);
            line += self.text[range.clone()].matches('\n').count();
            out.push_lines(&with);
            pos = range.end;
        }
        out.push_slice(self, pos..self.text.len(), line);
        *self = out;
    }

    /// [`Lines::edit`] of one range.
    pub fn replace(&mut self, range: Range<usize>, with: &str) {
        self.edit(vec![(range, with.to_owned())]);
    }

    /// [`Lines::edit_lines`] of one range.
    pub fn replace_lines(&mut self, range: Range<usize>, with: Lines) {
        self.edit_lines(vec![(range, with)]);
    }
}

/// A stage with its includes inlined ([`crate::expand_includes`]), and where its text came
/// from: the file id (`ns:path`), line and column. Rewriting it keeps them
/// ([`Expanded::edit`], [`Expanded::wrap`]).
#[derive(Clone, Debug)]
pub struct Expanded {
    pub(crate) lines: Lines,
    /// The stage file first, then each include, by id.
    pub(crate) files: Vec<String>,
    /// The GLSL version the stage file declares, what `__VERSION__` reads
    /// ([`crate::stage_version`]).
    pub(crate) version: u32,
}

impl Expanded {
    pub fn text(&self) -> &str {
        self.lines.text()
    }

    /// The stage's own file id.
    pub fn file(&self) -> &str {
        &self.files[0]
    }

    /// The id of each file pasted into the stage, in the order first read.
    pub fn includes(&self) -> &[String] {
        &self.files[1..]
    }

    /// The error at byte `at` of [`Expanded::text`], at its source file, line and column.
    /// Sunrise's own lines, or no byte, name the stage file with no line or column.
    pub fn error_at(&self, at: Option<usize>, message: String) -> Error {
        let pos = at.and_then(|at| self.lines.position(at));
        self.located_error(pos, || at.and_then(|at| self.lines.origin_at(at)), message)
    }

    /// [`Expanded::error_at`] the start of naga's `span` (none for an unknown one).
    pub fn error_in(&self, span: naga::Span, message: String) -> Error {
        self.error_at(span.to_range().map(|r| r.start), message)
    }

    /// Locate each of naga's parse errors in `stage`, indexing the rewritten text's line
    /// breaks once. `message` receives the error kind and the line naga read (the first line
    /// when the span is unknown); the diagnostic's position is mapped back to the author.
    pub fn errors_in(
        &self,
        stage: naga::ShaderStage,
        errors: &naga::front::glsl::ParseErrors,
        message: impl Fn(&naga::front::glsl::ErrorKind, &str) -> String,
    ) -> Vec<Error> {
        let text = self.text();
        let breaks: Vec<usize> = text.match_indices('\n').map(|(at, _)| at).collect();
        errors
            .errors
            .iter()
            .map(|error| {
                let at = error.meta.to_range().map(|r| r.start);
                let line = at.map_or(0, |at| breaks.partition_point(|&b| b < at));
                let from = line.checked_sub(1).map_or(0, |l| breaks[l] + 1);
                let to = breaks.get(line).copied().unwrap_or(text.len());
                self.located_error(
                    at.and_then(|at| self.lines.position(at)),
                    || at.and_then(|_| self.lines.origin(line)),
                    message(&error.kind, &text[from..to]),
                )
                .with_stage(stage)
            })
            .collect()
    }

    fn located_error(
        &self,
        pos: Option<Position>,
        origin: impl FnOnce() -> Option<Origin>,
        message: String,
    ) -> Error {
        let mut error = self
            .location(pos.map(|p| p.origin).or_else(origin))
            .error(message);
        error.column = pos.map_or(0, |p| p.column);
        error
    }

    /// The file and line of `origin`; the stage file with no line for Sunrise's own code.
    fn location(&self, origin: Option<Origin>) -> Location {
        match origin {
            Some(origin) => Location {
                file: self.files[origin.file as usize].clone(),
                line: origin.line,
            },
            None => Location {
                file: self.file().to_owned(),
                line: 0,
            },
        }
    }

    /// Where byte `at` of [`Expanded::text`] came from ([`Expanded::error_at`]).
    pub fn location_at(&self, at: Option<usize>) -> Location {
        self.location(at.and_then(|at| {
            self.lines
                .position(at)
                .map(|p| p.origin)
                .or_else(|| self.lines.origin_at(at))
        }))
    }

    /// Where the start of naga's `span` came from.
    pub fn location_in(&self, span: naga::Span) -> Location {
        self.location_at(span.to_range().map(|r| r.start))
    }

    /// Each range of [`Expanded::text`] (in order, apart) replaced by its text. A replacement
    /// with as many line breaks as the text it replaces is anchored to the start of each
    /// replaced piece, line for line; any other replacement is anchored to the start of its range.
    pub fn edit(&mut self, edits: Vec<(Range<usize>, String)>) {
        self.lines.edit(edits);
    }

    /// Sunrise's own code put `before` and `after` the text.
    pub fn wrap(&mut self, before: &str, after: &str) {
        let mut lines = Lines::uniform(before, None);
        lines.push_lines(&self.lines);
        lines.push(after, None);
        self.lines = lines;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(lines: &Lines, needle: &str) -> Option<u32> {
        lines
            .origin_at(lines.text().find(needle).expect("in the text"))
            .map(|o| o.line)
    }

    /// Text added in the middle of a line comes from that line; lines it adds too; the rest of
    /// the line keeps its own.
    #[test]
    fn an_edit_that_adds_lines_takes_its_starts_line() {
        let mut lines = Lines::from_text("a\nb X c\nd\n");
        let x = lines.text().find('X').unwrap();
        lines.replace(x..x + 1, "1\n2\n3");
        assert_eq!(lines.text(), "a\nb 1\n2\n3 c\nd\n");
        assert_eq!(
            [
                at(&lines, "a"),
                at(&lines, "b"),
                at(&lines, "2"),
                at(&lines, "3 c"),
                at(&lines, "d")
            ],
            [Some(1), Some(2), Some(2), Some(2), Some(3)]
        );
    }

    /// A replacement of as many lines as it replaces keeps their origins line for line; text
    /// removed with its line breaks leaves the next lines theirs.
    #[test]
    fn an_edit_of_the_same_lines_keeps_them_and_a_removal_keeps_the_rest() {
        let mut lines = Lines::from_text("a\nb\nc\nd\ne\n");
        let b = lines.text().find('b').unwrap();
        let d = lines.text().find('d').unwrap();
        lines.edit(vec![(b..b + 3, "B\nC".into()), (d..d + 2, String::new())]);
        assert_eq!(lines.text(), "a\nB\nC\ne\n");
        assert_eq!(
            [at(&lines, "B"), at(&lines, "C"), at(&lines, "e")],
            [Some(2), Some(3), Some(5)]
        );
    }

    /// Lines brought with their own origins keep them; the text after the range continues the
    /// last one, with its own origin when that line is empty.
    #[test]
    fn edited_lines_keep_their_origins() {
        let mut lines = Lines::from_text("a\nb\nc\n");
        let b = lines.text().find('b').unwrap();
        let mut with = Lines::uniform("p\n", Some(Origin { file: 1, line: 7 }));
        with.push("q\n", Some(Origin { file: 1, line: 8 }));
        lines.replace_lines(b..b, with);
        assert_eq!(lines.text(), "a\np\nq\nb\nc\n");
        assert_eq!(
            [at(&lines, "p"), at(&lines, "q"), at(&lines, "b")],
            [Some(7), Some(8), Some(2)]
        );
    }

    #[test]
    fn an_error_names_its_position_and_stage_when_known() {
        use naga::ShaderStage::{Compute, Fragment, Vertex};
        let error = |file, line, column, stage| Error {
            file,
            line,
            column,
            stage,
            message: "oops".into(),
            unprovided: None,
        };
        let file = "minecraft:shaders/include/shared.glsl";
        assert_eq!(
            describe(&[
                error(file.into(), 3, 5, Some(Fragment)),
                error(file.into(), 0, 0, Some(Vertex)),
                error(String::new(), 0, 0, Some(Compute)),
                error(file.into(), 3, 0, None),
            ]),
            "minecraft:shaders/include/shared.glsl:3:5: fragment shader: oops; \
             minecraft:shaders/include/shared.glsl: vertex shader: oops; \
             compute shader: oops; minecraft:shaders/include/shared.glsl:3: oops"
        );
    }
}
