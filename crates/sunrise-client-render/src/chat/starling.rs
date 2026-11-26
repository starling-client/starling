//! Starling's part of chat.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// Arguments that take any text (messages, strings, NBT): Enter never completes them.
pub(super) fn free_text(node: &crate::command::Node) -> bool {
    use crate::command::{ArgType as T, NodeKind};
    matches!(
        &node.kind,
        NodeKind::Argument {
            ty: T::Word
                | T::Str
                | T::Greedy
                | T::Message
                | T::Component
                | T::Style
                | T::Nbt
                | T::NbtTag
                | T::NbtPath,
            ..
        }
    )
}

/// Enter on a command that does not parse as typed (Sunrise; Bedrock takes `/gamemode c`): each
/// word that fails is completed when exactly one of the chat's completions starts with it, case
/// aside (as Tab would put it), and the completed command is what is sent when it then parses.
/// None, to send it as typed, for anything else: a command that parses already, a word with no
/// completion or several, free text, a word the server completes, or a completion that still
/// does not parse.
pub fn complete_command(c: &Commands, typed: &str) -> Option<String> {
    if !typed.starts_with('/') {
        return None;
    }
    let mut parse = c.dispatcher.parse_at(typed, 1, c.registries);
    if parse.executable().is_ok() {
        return None;
    }
    let mut line = typed.to_owned();
    // Each completion moves the failure on by a word at least.
    for _ in 0..typed.split(' ').count() {
        let start = parse.cursor;
        let end = line[start.min(line.len())..]
            .find(char::is_whitespace)
            .map_or(line.len(), |i| start + i);
        if start >= end {
            return None;
        }
        let (node, _, _) = parse.context.suggestion_context(end);
        let nodes = &c.dispatcher.nodes;
        if nodes[node].children.iter().any(|&n| free_text(&nodes[n])) {
            return None;
        }
        let (_, list, ask) = c
            .dispatcher
            .client_suggestions_asking(&parse, end, c.registries);
        let [only] = list.as_slice() else {
            return None;
        };
        if ask || only.range.0 < start || only.range.1 != end {
            return None;
        }
        let word = line[only.range.0..end].to_lowercase();
        let text = only.text.to_lowercase();
        let starts = text.starts_with(&word)
            || text
                .strip_prefix("minecraft:")
                .is_some_and(|t| t.starts_with(&word));
        if !starts || only.text.contains(char::is_whitespace) {
            return None;
        }
        line = format!("{}{}", only.apply(&line[..end]), &line[end..]);
        parse = c.dispatcher.parse_at(&line, 1, c.registries);
        if parse.executable().is_ok() {
            return (line.chars().count() <= 256).then_some(line);
        }
        if parse.cursor <= start {
            return None;
        }
    }
    None
}

impl ChatScreen {
    /// The whole input at once, as the on-screen keyboard's editor holds it (Android): the text
    /// without the characters typing refuses, cut to the maximum length, with the cursor and the
    /// selection's other end (in characters of `text`).
    pub fn set_text(
        &mut self,
        m: &Measure,
        c: &Commands,
        text: &str,
        cursor: usize,
        anchor: usize,
    ) {
        let allowed = |ch: char| ch >= ' ' && ch != '\u{7f}' && ch != '§';
        // Positions in the text kept.
        let kept = |pos: usize| text.chars().take(pos).filter(|&ch| allowed(ch)).count();
        let value: String = text
            .chars()
            .filter(|&ch| allowed(ch))
            .take(self.input.max_length)
            .collect();
        let changed = value != self.input.value;
        if changed {
            self.input.set_value(m, &value);
        }
        self.input.move_cursor_to(m, kept(anchor), false);
        self.input.move_cursor_to(m, kept(cursor), true);
        if changed {
            self.edited(m, c);
        }
    }
}
