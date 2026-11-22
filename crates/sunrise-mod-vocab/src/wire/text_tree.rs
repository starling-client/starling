//! Stack-independent traversal of the vanilla component tree in the shared Wire format.
use super::{PAYLOAD_MAX_DEPTH, Reader, Wire, WireError, Writer};
use crate::Tag;
use crate::text::{
    MAX_TEXT_DEPTH, Text, TextContent, TextHover, TextNbtSource, TextObject, TextProfile,
    validate_profile,
};

const MALFORMED_TEXT: WireError = WireError::Invalid("malformed text fields");

impl Wire for Text {
    // Minimum valid Text: Literal("") (8) + empty TextStyle with compound metadata (19)
    // + empty children list (4) + empty compound metadata (8).
    const MIN: usize = 39;
    layout_fields!(
        "validated-text-tree:v2";
        "content" => crate::text::TextContent,
        "style" => crate::text::TextStyle,
        "children" => Vec<crate::text::Text>,
        "metadata" => crate::Tag,
    );
    fn put(&self, out: &mut Writer) {
        let start = out.depth;
        put_tree(self, out, start);
        out.depth = start;
    }
    fn take(input: &mut Reader<'_>) -> Result<Self, WireError> {
        let start = input.depth;
        let result = take_tree(input, start);
        input.depth = start;
        result
    }
}

enum WriteFrame<'a> {
    Arguments {
        node: &'a Text,
        rest: std::slice::Iter<'a, Text>,
    },
    ContentChild {
        node: &'a Text,
    },
    HoverChild {
        node: &'a Text,
    },
    Children {
        node: &'a Text,
        rest: std::slice::Iter<'a, Text>,
    },
}

enum WriteAction<'a> {
    Enter(&'a Text),
    Style(&'a Text),
    StyleTail(&'a Text),
    Metadata(&'a Text),
}

fn put_text_list_len(out: &mut Writer, len: usize) -> bool {
    if out.error().is_some() {
        return false;
    }
    if out.payload.is_some() {
        let Some(count) = out.list_len(len) else {
            return false;
        };
        let Some(size) = Text::MIN
            .checked_mul(len)
            .and_then(|size| size.checked_add(4))
        else {
            out.fail(WireError::Length);
            return false;
        };
        if !out.can_fit(size)
            || !out.charge::<Text>(len)
            || (len != 0 && !out.charge::<DecodeFrame>(4))
        {
            return false;
        }
        if out.bytes.capacity() - out.bytes.len() < 4 {
            out.bytes.reserve(size);
        }
        out.bytes.extend_from_slice(&count.to_le_bytes());
        true
    } else {
        out.put_len(len);
        out.error().is_none()
    }
}

fn put_boxed_child_budget(out: &mut Writer) -> bool {
    out.error().is_none() && out.charge::<Text>(1) && out.charge::<DecodeFrame>(4)
}

fn put_tree(root: &Text, out: &mut Writer, start: u16) {
    let bounded = out.payload.is_some();
    let limit = if bounded {
        usize::from(PAYLOAD_MAX_DEPTH.saturating_sub(start))
    } else {
        MAX_TEXT_DEPTH
    };
    let mut frames: Vec<WriteFrame<'_>> = Vec::new();
    let mut action = WriteAction::Enter(root);

    loop {
        if out.error().is_some() {
            return;
        }
        out.depth = if bounded {
            start + frames.len() as u16
        } else {
            0
        };
        match action {
            WriteAction::Enter(node) => {
                if frames.len() >= limit {
                    out.fail(WireError::Depth);
                    return;
                }
                match &node.content {
                    TextContent::Literal(s) => {
                        out.put(&0u32);
                        out.put(s);
                        action = WriteAction::Style(node);
                    }
                    TextContent::Translatable {
                        key,
                        fallback,
                        arguments,
                    } => {
                        out.put(&1u32);
                        out.put(key);
                        out.put(fallback);
                        if !put_text_list_len(out, arguments.len()) {
                            return;
                        }
                        let mut iter = arguments.iter();
                        if let Some(first) = iter.next() {
                            frames.push(WriteFrame::Arguments { node, rest: iter });
                            action = WriteAction::Enter(first);
                        } else {
                            action = WriteAction::Style(node);
                        }
                    }
                    TextContent::Keybind(s) => {
                        out.put(&2u32);
                        out.put(s);
                        action = WriteAction::Style(node);
                    }
                    TextContent::Selector { pattern, separator } => {
                        out.put(&3u32);
                        out.put(pattern);
                        match separator {
                            None => {
                                out.put(&0u8);
                                action = WriteAction::Style(node);
                            }
                            Some(child) => {
                                out.put(&1u8);
                                if !put_boxed_child_budget(out) {
                                    return;
                                }
                                frames.push(WriteFrame::ContentChild { node });
                                action = WriteAction::Enter(child);
                            }
                        }
                    }
                    TextContent::Score { name, objective } => {
                        out.put(&4u32);
                        out.put(name);
                        out.put(objective);
                        action = WriteAction::Style(node);
                    }
                    TextContent::Nbt {
                        path,
                        interpret,
                        source,
                        separator,
                    } => {
                        out.put(&5u32);
                        out.put(path);
                        out.put(interpret);
                        out.put(source);
                        match separator {
                            None => {
                                out.put(&0u8);
                                action = WriteAction::Style(node);
                            }
                            Some(child) => {
                                out.put(&1u8);
                                if !put_boxed_child_budget(out) {
                                    return;
                                }
                                frames.push(WriteFrame::ContentChild { node });
                                action = WriteAction::Enter(child);
                            }
                        }
                    }
                    TextContent::Object(object) => {
                        out.put(&6u32);
                        match object {
                            TextObject::Player {
                                profile,
                                hat,
                                fallback,
                            } => {
                                if validate_profile(profile).is_err() {
                                    out.fail(MALFORMED_TEXT);
                                    return;
                                }
                                out.put(&0u32);
                                out.put(profile);
                                out.put(hat);
                                match fallback {
                                    None => {
                                        out.put(&0u8);
                                        action = WriteAction::Style(node);
                                    }
                                    Some(child) => {
                                        out.put(&1u8);
                                        if !put_boxed_child_budget(out) {
                                            return;
                                        }
                                        frames.push(WriteFrame::ContentChild { node });
                                        action = WriteAction::Enter(child);
                                    }
                                }
                            }
                            TextObject::Atlas {
                                atlas,
                                sprite,
                                fallback,
                            } => {
                                out.put(&1u32);
                                out.put(atlas);
                                out.put(sprite);
                                match fallback {
                                    None => {
                                        out.put(&0u8);
                                        action = WriteAction::Style(node);
                                    }
                                    Some(child) => {
                                        out.put(&1u8);
                                        if !put_boxed_child_budget(out) {
                                            return;
                                        }
                                        frames.push(WriteFrame::ContentChild { node });
                                        action = WriteAction::Enter(child);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            WriteAction::Style(node) => {
                let style = &node.style;
                out.put(&style.color);
                out.put(&style.shadow_color);
                out.put(&style.bold);
                out.put(&style.italic);
                out.put(&style.underlined);
                out.put(&style.strikethrough);
                out.put(&style.obfuscated);
                out.put(&style.click);
                match &style.hover {
                    None => {
                        out.put(&0u8);
                        action = WriteAction::StyleTail(node);
                    }
                    Some(TextHover::Text(child)) => {
                        out.put(&1u8);
                        out.put(&0u32);
                        if !put_boxed_child_budget(out) {
                            return;
                        }
                        frames.push(WriteFrame::HoverChild { node });
                        action = WriteAction::Enter(child);
                    }
                    Some(TextHover::Item {
                        id,
                        count,
                        components,
                    }) => {
                        out.put(&1u8);
                        out.put(&1u32);
                        out.put(id);
                        out.put(count);
                        out.put(components);
                        action = WriteAction::StyleTail(node);
                    }
                    Some(TextHover::Entity { kind, id, name }) => {
                        out.put(&1u8);
                        out.put(&2u32);
                        out.put(kind);
                        out.put(id);
                        match name {
                            None => {
                                out.put(&0u8);
                                action = WriteAction::StyleTail(node);
                            }
                            Some(child) => {
                                out.put(&1u8);
                                if !put_boxed_child_budget(out) {
                                    return;
                                }
                                frames.push(WriteFrame::HoverChild { node });
                                action = WriteAction::Enter(child);
                            }
                        }
                    }
                }
            }
            WriteAction::StyleTail(node) => {
                let style = &node.style;
                out.put(&style.insertion);
                out.put(&style.font);
                if !matches!(style.metadata, Tag::Compound(_)) {
                    out.fail(MALFORMED_TEXT);
                    return;
                }
                out.put(&style.metadata);
                if !put_text_list_len(out, node.children.len()) {
                    return;
                }
                let mut iter = node.children.iter();
                if let Some(first) = iter.next() {
                    frames.push(WriteFrame::Children { node, rest: iter });
                    action = WriteAction::Enter(first);
                } else {
                    action = WriteAction::Metadata(node);
                }
            }
            WriteAction::Metadata(node) => {
                if !matches!(node.metadata, Tag::Compound(_)) {
                    out.fail(MALFORMED_TEXT);
                    return;
                }
                out.put(&node.metadata);
                if out.error().is_some() {
                    return;
                }
                match frames.pop() {
                    None => return,
                    Some(WriteFrame::Arguments { node, mut rest }) => {
                        if let Some(next) = rest.next() {
                            frames.push(WriteFrame::Arguments { node, rest });
                            action = WriteAction::Enter(next);
                        } else {
                            action = WriteAction::Style(node);
                        }
                    }
                    Some(WriteFrame::ContentChild { node }) => {
                        action = WriteAction::Style(node);
                    }
                    Some(WriteFrame::HoverChild { node }) => {
                        action = WriteAction::StyleTail(node);
                    }
                    Some(WriteFrame::Children { node, mut rest }) => {
                        if let Some(next) = rest.next() {
                            frames.push(WriteFrame::Children { node, rest });
                            action = WriteAction::Enter(next);
                        } else {
                            action = WriteAction::Metadata(node);
                        }
                    }
                }
            }
        }
    }
}

enum DecodeFrame {
    Arguments {
        key: String,
        fallback: Option<String>,
        remaining: usize,
        arguments: Vec<Text>,
    },
    SelectorSeparator {
        pattern: String,
    },
    NbtSeparator {
        path: String,
        interpret: bool,
        source: TextNbtSource,
    },
    PlayerFallback(Text),
    AtlasFallback {
        atlas: String,
        sprite: String,
    },
    HoverText(Text),
    HoverEntityName {
        node: Text,
        kind: String,
        id: [u64; 2],
    },
    Children {
        node: Text,
        remaining: usize,
    },
}

enum DecodeAction {
    Enter,
    Style(Text),
    StyleTail(Text),
    Metadata(Text),
}

fn take_text_list_len(input: &mut Reader<'_>) -> Result<usize, WireError> {
    let len = input.take_len(Text::MIN)?;
    input.charge::<Text>(len)?;
    if len != 0 {
        input.charge::<DecodeFrame>(4)?;
    }
    Ok(len)
}

fn take_boxed_child_budget(input: &mut Reader<'_>) -> Result<(), WireError> {
    input.charge::<Text>(1)?;
    input.charge::<DecodeFrame>(4)
}

fn take_tree(input: &mut Reader<'_>, start: u16) -> Result<Text, WireError> {
    let bounded = input.budget.is_some();
    let limit = if bounded {
        usize::from(PAYLOAD_MAX_DEPTH.saturating_sub(start))
    } else {
        MAX_TEXT_DEPTH
    };
    let mut frames: Vec<DecodeFrame> = Vec::new();
    let mut action = DecodeAction::Enter;

    loop {
        input.depth = if bounded {
            start + frames.len() as u16
        } else {
            0
        };
        match action {
            DecodeAction::Enter => {
                if frames.len() >= limit {
                    return Err(WireError::Depth);
                }
                match input.take::<u32>()? {
                    0 => {
                        let s = input.take()?;
                        action = DecodeAction::Style(Text::new(TextContent::Literal(s)));
                    }
                    1 => {
                        let key = input.take()?;
                        let fallback = input.take()?;
                        let len = take_text_list_len(input)?;
                        if len == 0 {
                            action = DecodeAction::Style(Text::new(TextContent::Translatable {
                                key,
                                fallback,
                                arguments: Vec::new(),
                            }));
                        } else {
                            frames.push(DecodeFrame::Arguments {
                                key,
                                fallback,
                                remaining: len,
                                arguments: Vec::with_capacity(len),
                            });
                            action = DecodeAction::Enter;
                        }
                    }
                    2 => {
                        let s = input.take()?;
                        action = DecodeAction::Style(Text::new(TextContent::Keybind(s)));
                    }
                    3 => {
                        let pattern = input.take()?;
                        match u8::take(input)? {
                            0 => {
                                action = DecodeAction::Style(Text::new(TextContent::Selector {
                                    pattern,
                                    separator: None,
                                }));
                            }
                            1 => {
                                take_boxed_child_budget(input)?;
                                frames.push(DecodeFrame::SelectorSeparator { pattern });
                                action = DecodeAction::Enter;
                            }
                            other => return Err(WireError::Tag(other.into())),
                        }
                    }
                    4 => {
                        let name = input.take()?;
                        let objective = input.take()?;
                        action =
                            DecodeAction::Style(Text::new(TextContent::Score { name, objective }));
                    }
                    5 => {
                        let path = input.take()?;
                        let interpret = input.take()?;
                        let source = input.take()?;
                        match u8::take(input)? {
                            0 => {
                                action = DecodeAction::Style(Text::new(TextContent::Nbt {
                                    path,
                                    interpret,
                                    source,
                                    separator: None,
                                }));
                            }
                            1 => {
                                take_boxed_child_budget(input)?;
                                frames.push(DecodeFrame::NbtSeparator {
                                    path,
                                    interpret,
                                    source,
                                });
                                action = DecodeAction::Enter;
                            }
                            other => return Err(WireError::Tag(other.into())),
                        }
                    }
                    6 => match input.take::<u32>()? {
                        0 => {
                            let profile: TextProfile = input.take()?;
                            let valid = validate_profile(&profile).is_ok();
                            // Wrap in `Text` before failing so `Text::drop` releases nested profile tags iteratively.
                            let mut parent = Text::new(TextContent::Object(TextObject::Player {
                                profile,
                                hat: false,
                                fallback: None,
                            }));
                            if !valid {
                                return Err(MALFORMED_TEXT);
                            }
                            let hat = input.take()?;
                            if let TextContent::Object(TextObject::Player { hat: slot, .. }) =
                                &mut parent.content
                            {
                                *slot = hat;
                            }
                            match u8::take(input)? {
                                0 => action = DecodeAction::Style(parent),
                                1 => {
                                    take_boxed_child_budget(input)?;
                                    frames.push(DecodeFrame::PlayerFallback(parent));
                                    action = DecodeAction::Enter;
                                }
                                other => return Err(WireError::Tag(other.into())),
                            }
                        }
                        1 => {
                            let atlas = input.take()?;
                            let sprite = input.take()?;
                            match u8::take(input)? {
                                0 => {
                                    action = DecodeAction::Style(Text::new(TextContent::Object(
                                        TextObject::Atlas {
                                            atlas,
                                            sprite,
                                            fallback: None,
                                        },
                                    )));
                                }
                                1 => {
                                    take_boxed_child_budget(input)?;
                                    frames.push(DecodeFrame::AtlasFallback { atlas, sprite });
                                    action = DecodeAction::Enter;
                                }
                                other => return Err(WireError::Tag(other.into())),
                            }
                        }
                        other => return Err(WireError::Tag(other)),
                    },
                    other => return Err(WireError::Tag(other)),
                }
            }
            DecodeAction::Style(mut node) => {
                node.style.color = input.take()?;
                node.style.shadow_color = input.take()?;
                node.style.bold = input.take()?;
                node.style.italic = input.take()?;
                node.style.underlined = input.take()?;
                node.style.strikethrough = input.take()?;
                node.style.obfuscated = input.take()?;
                node.style.click = input.take()?;
                match u8::take(input)? {
                    0 => action = DecodeAction::StyleTail(node),
                    1 => match input.take::<u32>()? {
                        0 => {
                            take_boxed_child_budget(input)?;
                            frames.push(DecodeFrame::HoverText(node));
                            action = DecodeAction::Enter;
                        }
                        1 => {
                            let id = input.take()?;
                            let count = input.take()?;
                            let components = input.take()?;
                            node.style.hover = Some(TextHover::Item {
                                id,
                                count,
                                components,
                            });
                            action = DecodeAction::StyleTail(node);
                        }
                        2 => {
                            let kind = input.take()?;
                            let id = input.take()?;
                            match u8::take(input)? {
                                0 => {
                                    node.style.hover = Some(TextHover::Entity {
                                        kind,
                                        id,
                                        name: None,
                                    });
                                    action = DecodeAction::StyleTail(node);
                                }
                                1 => {
                                    take_boxed_child_budget(input)?;
                                    frames.push(DecodeFrame::HoverEntityName { node, kind, id });
                                    action = DecodeAction::Enter;
                                }
                                other => return Err(WireError::Tag(other.into())),
                            }
                        }
                        other => return Err(WireError::Tag(other)),
                    },
                    other => return Err(WireError::Tag(other.into())),
                }
            }
            DecodeAction::StyleTail(mut node) => {
                node.style.insertion = input.take()?;
                node.style.font = input.take()?;
                let metadata: Tag = input.take()?;
                if !matches!(metadata, Tag::Compound(_)) {
                    sunrise_core::nbt::discard_tag(metadata);
                    return Err(MALFORMED_TEXT);
                }
                node.style.metadata = metadata;
                let len = take_text_list_len(input)?;
                if len == 0 {
                    action = DecodeAction::Metadata(node);
                } else {
                    node.children = Vec::with_capacity(len);
                    frames.push(DecodeFrame::Children {
                        node,
                        remaining: len,
                    });
                    action = DecodeAction::Enter;
                }
            }
            DecodeAction::Metadata(mut node) => {
                let metadata: Tag = input.take()?;
                if !matches!(metadata, Tag::Compound(_)) {
                    sunrise_core::nbt::discard_tag(metadata);
                    return Err(MALFORMED_TEXT);
                }
                node.metadata = metadata;
                match frames.pop() {
                    None => return Ok(node),
                    Some(DecodeFrame::Arguments {
                        key,
                        fallback,
                        remaining,
                        mut arguments,
                    }) => {
                        arguments.push(node);
                        let remaining = remaining - 1;
                        if remaining != 0 {
                            frames.push(DecodeFrame::Arguments {
                                key,
                                fallback,
                                remaining,
                                arguments,
                            });
                            action = DecodeAction::Enter;
                        } else {
                            action = DecodeAction::Style(Text::new(TextContent::Translatable {
                                key,
                                fallback,
                                arguments,
                            }));
                        }
                    }
                    Some(DecodeFrame::SelectorSeparator { pattern }) => {
                        action = DecodeAction::Style(Text::new(TextContent::Selector {
                            pattern,
                            separator: Some(Box::new(node)),
                        }));
                    }
                    Some(DecodeFrame::NbtSeparator {
                        path,
                        interpret,
                        source,
                    }) => {
                        action = DecodeAction::Style(Text::new(TextContent::Nbt {
                            path,
                            interpret,
                            source,
                            separator: Some(Box::new(node)),
                        }));
                    }
                    Some(DecodeFrame::PlayerFallback(mut parent)) => {
                        if let TextContent::Object(TextObject::Player { fallback, .. }) =
                            &mut parent.content
                        {
                            *fallback = Some(Box::new(node));
                        }
                        action = DecodeAction::Style(parent);
                    }
                    Some(DecodeFrame::AtlasFallback { atlas, sprite }) => {
                        action = DecodeAction::Style(Text::new(TextContent::Object(
                            TextObject::Atlas {
                                atlas,
                                sprite,
                                fallback: Some(Box::new(node)),
                            },
                        )));
                    }
                    Some(DecodeFrame::HoverText(mut parent)) => {
                        parent.style.hover = Some(TextHover::Text(Box::new(node)));
                        action = DecodeAction::StyleTail(parent);
                    }
                    Some(DecodeFrame::HoverEntityName {
                        node: mut parent,
                        kind,
                        id,
                    }) => {
                        parent.style.hover = Some(TextHover::Entity {
                            kind,
                            id,
                            name: Some(Box::new(node)),
                        });
                        action = DecodeAction::StyleTail(parent);
                    }
                    Some(DecodeFrame::Children {
                        node: mut parent,
                        remaining,
                    }) => {
                        parent.children.push(node);
                        let remaining = remaining - 1;
                        if remaining != 0 {
                            frames.push(DecodeFrame::Children {
                                node: parent,
                                remaining,
                            });
                            action = DecodeAction::Enter;
                        } else {
                            action = DecodeAction::Metadata(parent);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::TextStyle;

    #[test]
    fn structured_text_encodes_fields_in_declaration_order() {
        const EMPTY_COMPOUND: &[u8] = &[10, 0, 0, 0, 0, 0, 0, 0];
        let literal_bytes = |letter: u8| {
            let mut bytes = vec![0, 0, 0, 0, 1, 0, 0, 0, letter]; // Literal(1-byte)
            bytes.extend([0; 11]); // Unset style options.
            bytes.extend_from_slice(EMPTY_COMPOUND); // Style metadata.
            bytes.extend([0, 0, 0, 0]); // Empty children.
            bytes.extend_from_slice(EMPTY_COMPOUND); // Component metadata.
            bytes
        };

        let text = Text::translate("k", vec![Text::literal("a")])
            .append(Text::literal("c"))
            .with_style(TextStyle {
                bold: Some(false),
                hover: Some(TextHover::Text(Box::new(Text::literal("h")))),
                ..TextStyle::default()
            });

        let mut bytes = Vec::new();
        // content: Translatable("k", None, [Literal("a")])
        bytes.extend([1, 0, 0, 0, 1, 0, 0, 0, b'k', 0, 1, 0, 0, 0]);
        bytes.extend(literal_bytes(b'a'));
        // style: color=None, shadow_color=None, bold=Some(false), italic..click=None
        bytes.extend([0, 0, 1, 0, 0, 0, 0, 0, 0]);
        // hover: Some(TextHover::Text(Literal("h")))
        bytes.extend([1, 0, 0, 0, 0]);
        bytes.extend(literal_bytes(b'h'));
        // insertion=None, font=None, style.metadata={}
        bytes.extend([0, 0]);
        bytes.extend_from_slice(EMPTY_COMPOUND);
        // children: [Literal("c")]
        bytes.extend([1, 0, 0, 0]);
        bytes.extend(literal_bytes(b'c'));
        // metadata: {}
        bytes.extend_from_slice(EMPTY_COMPOUND);

        let mut out = Writer::new();
        out.put(&text);
        assert_eq!(out.error(), None);
        assert_eq!(out.bytes(), bytes.as_slice());
        assert_eq!(Reader::decode::<Text>(&bytes).unwrap(), text);

        // Minimal valid Text matches Text::MIN exactly.
        let mut minimal = Writer::new();
        minimal.put(&Text::literal(""));
        assert_eq!(minimal.error(), None);
        assert_eq!(minimal.bytes().len(), Text::MIN);
    }
}
