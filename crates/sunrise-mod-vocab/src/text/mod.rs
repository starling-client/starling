//! Owned component trees. No language is resolved until the game displays a component.
use crate::Tag;
use sunrise_core::nbt::clone_tag;

/// A proposed substitution in a resolved text run, for [`crate::Hooks::substitute_inline_images`].
// exhaustive: a plain byte range and sprite id that mods build directly as a literal, like vanilla's records.
#[derive(Clone, Debug, PartialEq, Eq, sunrise_mod_api_macros::Wire)]
pub struct InlineImageReplacement {
    /// Inclusive start byte in the run.
    pub start: u32,
    /// Exclusive end byte in the run. Both ends must be UTF-8 character boundaries.
    pub end: u32,
    /// Sprite id in `minecraft:gui`, including a dynamic texture id.
    pub sprite: crate::Identifier,
}

/// A component tree, retaining translation keys, styles and ordered children.
/// Build it directly with [`Text::literal`], [`Text::translate`],
/// [`Text::translate_with_fallback`] or [`Text::new`] and lend it to host reads. The host validates it at the boundary.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
// no wire: handwritten Wire uses a stack-independent validated iterative codec.
pub struct Text {
    /// The component's contents before its children.
    pub content: TextContent,
    /// Fields not set here inherit from the enclosing component.
    pub style: TextStyle,
    /// Components appended in order.
    pub children: Vec<Text>,
    /// Unrecognised component metadata survives an engine round trip, as a compound.
    pub metadata: Tag,
}

/// What supplies the characters or inline object of a component.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: closed serialized component vocabulary; new cases change the link contract
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TextContent {
    /// Literal characters, without interpreting legacy formatting codes.
    Literal(String),
    /// A translation remains a key and components, not a localized string.
    Translatable {
        /// Translation identifier.
        key: String,
        /// Template used when the key is not present in the language.
        fallback: Option<String>,
        /// Ordered `%s` arguments, retaining their own styles.
        arguments: Vec<Text>,
    },
    /// A key binding's localized display name.
    Keybind(String),
    /// A selector resolved by the game's component context.
    Selector {
        /// Selector expression.
        pattern: String,
        /// Component between the selected names.
        separator: Option<Box<Text>>,
    },
    /// A scoreboard value resolved by the game's component context.
    Score {
        /// Score holder or selector.
        name: String,
        /// Scoreboard objective.
        objective: String,
    },
    /// NBT resolved by the game's component context.
    Nbt {
        /// NBT path expression.
        path: String,
        /// Whether the selected strings contain components.
        interpret: bool,
        /// Where to obtain the NBT.
        source: TextNbtSource,
        /// Component between selected values.
        separator: Option<Box<Text>>,
    },
    /// A non-character inline object.
    Object(TextObject),
}

/// The source of an NBT component, not the NBT itself.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: closed serialized component vocabulary; new cases change the link contract
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TextNbtSource {
    /// Block position expression.
    Block(String),
    /// Entity selector expression.
    Entity(String),
    /// Command storage identifier.
    Storage(String),
}

/// An inline object embedded in text.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: closed serialized component vocabulary; new cases change the link contract
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TextObject {
    /// A player head, retaining the profile properties carried by the component.
    Player {
        /// Profile data, including signed skin properties when present.
        profile: TextProfile,
        /// Whether to draw the hat layer.
        hat: bool,
        /// Optional component for consumers unable to display the object; None uses the default.
        fallback: Option<Box<Text>>,
    },
    /// A sprite in a named atlas.
    Atlas {
        /// Atlas identifier.
        atlas: String,
        /// Sprite identifier.
        sprite: String,
        /// Optional component for consumers unable to display the object; None uses the default.
        fallback: Option<Box<Text>>,
    },
}

/// Optional style fields inherit; explicit `false` overrides an inherited decoration.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TextStyle {
    /// Named colour or `#RRGGBB` component colour.
    pub color: Option<String>,
    /// Packed ARGB shadow colour; zero suppresses shadows.
    pub shadow_color: Option<u32>,
    /// Bold decoration.
    pub bold: Option<bool>,
    /// Italic decoration.
    pub italic: Option<bool>,
    /// Underline decoration.
    pub underlined: Option<bool>,
    /// Strikethrough decoration.
    pub strikethrough: Option<bool>,
    /// Obfuscated decoration.
    pub obfuscated: Option<bool>,
    /// Action invoked by clicking the component.
    pub click: Option<TextClick>,
    /// Information displayed on hover.
    pub hover: Option<TextHover>,
    /// Text inserted into the chat input on Shift-click.
    pub insertion: Option<String>,
    /// Font identifier.
    pub font: Option<String>,
    /// Unknown style fields, as a compound, without engine types.
    pub metadata: Tag,
}

/// A text component's click action.
///
/// File-opening actions are intentionally absent: mods can neither produce nor
/// round-trip `OpenFile`. Capture drops untrusted file-opening action metadata.
#[derive(Debug, PartialEq, Eq)]
// exhaustive: closed serialized component vocabulary; new cases change the link contract
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TextClick {
    /// Open an HTTP or HTTPS URL.
    OpenUrl(String),
    /// Switch to the numbered page of a book.
    ChangePage(i32),
    /// Put a command in the chat input.
    SuggestCommand(String),
    /// Run a command.
    RunCommand(String),
    /// Copy text to the clipboard.
    CopyToClipboard(String),
    /// Show a dialog identifier or inline declaration.
    ShowDialog(Tag),
    /// A server-defined action with optional payload.
    Custom {
        /// Action identifier.
        id: String,
        /// Payload retained without interpretation.
        payload: Option<Tag>,
    },
}

/// A text component's hover action.
#[derive(Debug, PartialEq)]
// exhaustive: closed serialized component vocabulary; new cases change the link contract
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TextHover {
    /// Show another styled component tree.
    Text(Box<Text>),
    /// Show an item with its component patch.
    Item {
        /// Item identifier, not a process-local registry slot.
        id: String,
        /// Display count.
        count: u32,
        /// Component patch carried by the component, if any.
        components: Option<Tag>,
    },
    /// Show an entity's identity and optional name.
    Entity {
        /// Entity type identifier.
        kind: String,
        /// UUID as the high and low halves.
        id: [u64; 2],
        /// Optional styled entity name.
        name: Option<Box<Text>>,
    },
}

impl Default for Text {
    fn default() -> Self {
        Self::literal("")
    }
}
impl Text {
    /// A component with the given contents, with no style overrides or children.
    pub fn new(content: TextContent) -> Self {
        Self {
            content,
            style: TextStyle::default(),
            children: Vec::new(),
            metadata: Tag::Compound(Default::default()),
        }
    }
    /// Check vanilla's component-depth bound before cloning any node payload.
    /// Counts every recursive component relation, including hover names.
    pub fn validate_depth(&self) -> Result<(), TextFormError> {
        let mut pending = vec![(self, 1)];
        while let Some((text, depth)) = pending.pop() {
            if depth > MAX_TEXT_DEPTH {
                return Err(TextFormError::TooDeep);
            }
            text.visit_children(|child| pending.push((child, depth + 1)));
        }
        Ok(())
    }
    /// A literal component, with no inherited style overrides or children.
    pub fn literal(value: impl Into<String>) -> Self {
        Self::new(TextContent::Literal(value.into()))
    }
    /// A translatable component. The key and arguments remain unresolved data.
    pub fn translate(key: impl Into<String>, arguments: Vec<Text>) -> Self {
        Self::new(TextContent::Translatable {
            key: key.into(),
            fallback: None,
            arguments,
        })
    }
    /// A translatable component with a fallback template (`Component.translatableWithFallback`):
    /// a language without `key` formats `fallback` with the arguments instead of showing the key.
    /// The key, fallback and arguments remain unresolved data.
    pub fn translate_with_fallback(
        key: impl Into<String>,
        fallback: impl Into<String>,
        arguments: Vec<Text>,
    ) -> Self {
        Self::new(TextContent::Translatable {
            key: key.into(),
            fallback: Some(fallback.into()),
            arguments,
        })
    }
    /// Append a child without flattening its style or translation.
    pub fn append(mut self, child: Text) -> Self {
        self.children.push(child);
        self
    }
    /// Set this component's style; children continue to inherit unset fields.
    pub fn with_style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }
}

/// A player profile, including ordered signed properties and unknown data.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TextProfile {
    /// Player name in U+0021..U+007E, at most 16 characters (ExtraCodecs.PLAYER_NAME).
    pub name: Option<String>,
    /// UUID as high and low halves.
    pub id: Option<[u64; 2]>,
    /// Ordered signed records; unsigned map values retain their order within each name.
    pub properties: Vec<TextProfileProperty>,
    /// Unknown profile fields, as a compound.
    pub metadata: Tag,
}
/// One signed or unsigned profile property.
///
/// ExtraCodecs.PROPERTY_MAP accepts at most 16 distinct unsigned map names.
/// Signed or metadata-bearing records instead use its 16-entry list codec,
/// whose UTF-16 limits are 64 units for names, 32767 for values and 1024 for signatures.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub struct TextProfileProperty {
    /// Property name.
    pub name: String,
    /// Property value.
    pub value: String,
    /// Optional signature.
    pub signature: Option<String>,
    /// Unknown property fields, as a compound.
    pub metadata: Tag,
}
impl Default for TextProfile {
    fn default() -> Self {
        Self {
            name: None,
            id: None,
            properties: Vec::new(),
            metadata: Tag::Compound(Default::default()),
        }
    }
}
impl Default for TextProfileProperty {
    fn default() -> Self {
        Self {
            name: String::new(),
            value: String::new(),
            signature: None,
            metadata: Tag::Compound(Default::default()),
        }
    }
}
impl Default for TextStyle {
    fn default() -> Self {
        Self {
            color: None,
            shadow_color: None,
            bold: None,
            italic: None,
            underlined: None,
            strikethrough: None,
            obfuscated: None,
            click: None,
            hover: None,
            insertion: None,
            font: None,
            metadata: Tag::Compound(Default::default()),
        }
    }
}

/// Maximum component depth, counted in nodes.
pub const MAX_TEXT_DEPTH: usize = 512;

/// A refused Text crossing. No partially converted fields are written.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum TextFormError {
    /// No backend is available.
    Unavailable,
    /// A component path or opaque NBT container path exceeds 512.
    TooDeep,
    /// The component has inconsistent or invalid fields.
    Malformed,
    /// Tag→JSON emission cannot represent NaN or infinity.
    NonFinite,
    /// Tag→JSON emission cannot represent an unpaired UTF-16 surrogate.
    InvalidString,
    /// Vanilla JsonOps→NbtOps refuses null nested in metadata.
    NestedNull,
}
impl std::fmt::Display for TextFormError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "text backend unavailable",
            Self::TooDeep => "text or NBT nesting exceeds maximum depth",
            Self::Malformed => "malformed text fields",
            Self::NonFinite => "JSON cannot represent a non-finite NBT number",
            Self::InvalidString => "JSON cannot represent an unpaired UTF-16 surrogate",
            Self::NestedNull => "JSON-to-NBT does not accept nested null",
        })
    }
}
impl std::error::Error for TextFormError {}
impl crate::NoBackend for TextFormError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}

impl Clone for TextClick {
    fn clone(&self) -> Self {
        match self {
            Self::OpenUrl(value) => Self::OpenUrl(value.clone()),
            Self::ChangePage(page) => Self::ChangePage(*page),
            Self::SuggestCommand(value) => Self::SuggestCommand(value.clone()),
            Self::RunCommand(value) => Self::RunCommand(value.clone()),
            Self::CopyToClipboard(value) => Self::CopyToClipboard(value.clone()),
            Self::ShowDialog(tag) => Self::ShowDialog(clone_tag(tag)),
            Self::Custom { id, payload } => Self::Custom {
                id: id.clone(),
                payload: payload.as_ref().map(clone_tag),
            },
        }
    }
}

impl TextHover {
    fn clone_with(&self, mut text: impl FnMut(&Text) -> Text) -> Self {
        match self {
            Self::Text(child) => Self::Text(Box::new(text(child))),
            Self::Item {
                id,
                count,
                components,
            } => Self::Item {
                id: id.clone(),
                count: *count,
                components: components.as_ref().map(clone_tag),
            },
            Self::Entity { kind, id, name } => Self::Entity {
                kind: kind.clone(),
                id: *id,
                name: name.as_deref().map(|name| Box::new(text(name))),
            },
        }
    }
}

impl Clone for TextHover {
    fn clone(&self) -> Self {
        self.clone_with(Text::clone)
    }
}

impl TextStyle {
    fn clone_with(&self, text: impl FnMut(&Text) -> Text) -> Self {
        Self {
            color: self.color.clone(),
            shadow_color: self.shadow_color,
            bold: self.bold,
            italic: self.italic,
            underlined: self.underlined,
            strikethrough: self.strikethrough,
            obfuscated: self.obfuscated,
            click: self.click.clone(),
            hover: self.hover.as_ref().map(|hover| hover.clone_with(text)),
            insertion: self.insertion.clone(),
            font: self.font.clone(),
            metadata: clone_tag(&self.metadata),
        }
    }
}

impl Clone for TextStyle {
    fn clone(&self) -> Self {
        self.clone_with(Text::clone)
    }
}

impl Clone for Text {
    fn clone(&self) -> Self {
        // Clone from the leaves up; the same traversal includes each hover name,
        // translation argument, separator and inline-object fallback.
        let mut work = vec![(self, false)];
        let mut ready = std::collections::HashMap::new();
        while let Some((node, done)) = work.pop() {
            if !done {
                work.push((node, true));
                node.visit_children(|child| work.push((child, false)));
                continue;
            }
            let mut take = |child: &Text| {
                ready
                    .remove(&std::ptr::from_ref(child))
                    .expect("cloned child")
            };
            let content = match &node.content {
                TextContent::Literal(value) => TextContent::Literal(value.clone()),
                TextContent::Keybind(key) => TextContent::Keybind(key.clone()),
                TextContent::Score { name, objective } => TextContent::Score {
                    name: name.clone(),
                    objective: objective.clone(),
                },
                TextContent::Translatable {
                    key,
                    fallback,
                    arguments,
                } => TextContent::Translatable {
                    key: key.clone(),
                    fallback: fallback.clone(),
                    arguments: arguments.iter().map(&mut take).collect(),
                },
                TextContent::Selector { pattern, separator } => TextContent::Selector {
                    pattern: pattern.clone(),
                    separator: separator.as_deref().map(|child| Box::new(take(child))),
                },
                TextContent::Nbt {
                    path,
                    interpret,
                    source,
                    separator,
                } => TextContent::Nbt {
                    path: path.clone(),
                    interpret: *interpret,
                    source: source.clone(),
                    separator: separator.as_deref().map(|child| Box::new(take(child))),
                },
                TextContent::Object(TextObject::Atlas {
                    atlas,
                    sprite,
                    fallback,
                }) => TextContent::Object(TextObject::Atlas {
                    atlas: atlas.clone(),
                    sprite: sprite.clone(),
                    fallback: fallback.as_deref().map(|child| Box::new(take(child))),
                }),
                TextContent::Object(TextObject::Player {
                    profile,
                    hat,
                    fallback,
                }) => TextContent::Object(TextObject::Player {
                    profile: TextProfile {
                        name: profile.name.clone(),
                        id: profile.id,
                        properties: profile
                            .properties
                            .iter()
                            .map(|p| TextProfileProperty {
                                name: p.name.clone(),
                                value: p.value.clone(),
                                signature: p.signature.clone(),
                                metadata: clone_tag(&p.metadata),
                            })
                            .collect(),
                        metadata: clone_tag(&profile.metadata),
                    },
                    hat: *hat,
                    fallback: fallback.as_deref().map(|child| Box::new(take(child))),
                }),
            };
            let style = node.style.clone_with(&mut take);
            let children = node.children.iter().map(take).collect();
            ready.insert(
                std::ptr::from_ref(node),
                Self {
                    content,
                    style,
                    children,
                    metadata: clone_tag(&node.metadata),
                },
            );
        }
        ready
            .remove(&std::ptr::from_ref(self))
            .expect("cloned root")
    }
}

fn discard_nested_tag(tag: &mut Tag) {
    if matches!(tag, Tag::List(v) if !v.is_empty())
        || matches!(tag, Tag::Compound(m) if !m.is_empty())
    {
        sunrise_core::nbt::discard_tag(std::mem::replace(tag, Tag::End));
    }
}

/// Drain the opaque tags before a style is freed. Nested hover text has its own iterative drop.
pub(crate) fn discard_style_tags(style: &mut TextStyle) {
    discard_nested_tag(&mut style.metadata);
    if let Some(
        TextClick::ShowDialog(tag)
        | TextClick::Custom {
            payload: Some(tag), ..
        },
    ) = &mut style.click
    {
        discard_nested_tag(tag);
    }
    if let Some(TextHover::Item {
        components: Some(tag),
        ..
    }) = &mut style.hover
    {
        discard_nested_tag(tag);
    }
}

fn drain_text_node(node: &mut Text, stack: &mut Vec<Text>) {
    discard_nested_tag(&mut node.metadata);
    discard_style_tags(&mut node.style);
    stack.append(&mut node.children);
    match &mut node.content {
        TextContent::Translatable { arguments, .. } => stack.append(arguments),
        TextContent::Selector { separator, .. } | TextContent::Nbt { separator, .. } => {
            if let Some(child) = separator.take() {
                stack.push(*child);
            }
        }
        TextContent::Object(TextObject::Player {
            profile, fallback, ..
        }) => {
            discard_nested_tag(&mut profile.metadata);
            for property in &mut profile.properties {
                discard_nested_tag(&mut property.metadata);
            }
            if let Some(child) = fallback.take() {
                stack.push(*child);
            }
        }
        TextContent::Object(TextObject::Atlas { fallback, .. }) => {
            if let Some(child) = fallback.take() {
                stack.push(*child);
            }
        }
        TextContent::Literal(_) | TextContent::Keybind(_) | TextContent::Score { .. } => {}
    }
    match &mut node.style.hover {
        Some(TextHover::Text(_)) => {
            if let Some(TextHover::Text(child)) = node.style.hover.take() {
                stack.push(*child);
            }
        }
        Some(TextHover::Entity { name, .. }) => {
            if let Some(child) = name.take() {
                stack.push(*child);
            }
        }
        Some(TextHover::Item { .. }) | None => {}
    }
}

impl Drop for Text {
    fn drop(&mut self) {
        let mut stack = Vec::new();
        drain_text_node(self, &mut stack);
        while let Some(mut node) = stack.pop() {
            drain_text_node(&mut node, &mut stack);
        }
    }
}

impl Text {
    /// Validate component paths, opaque NBT containers and typed profile fields
    /// before constructing owned conversion payloads. This does not impose JSON
    /// representation restrictions on NBT data; engine emission may refuse those.
    pub fn validate(&self) -> Result<(), TextFormError> {
        self.validate_depth()?;
        let mut pending = vec![self];
        while let Some(text) = pending.pop() {
            let profile = match &text.content {
                TextContent::Object(TextObject::Player { profile, .. }) => Some(profile),
                _ => None,
            };
            let item = match &text.style.hover {
                Some(TextHover::Item { components, .. }) => components.as_ref(),
                _ => None,
            };
            validate_payload(
                &text.metadata,
                &text.style.metadata,
                text.style.click.as_ref(),
                item,
                profile,
            )?;
            text.visit_children(|child| pending.push(child));
        }
        Ok(())
    }
    /// Lend each directly nested component in order: appended children first,
    /// then the content's arguments, separator or fallback, then hover text or
    /// a hover entity name. This does not descend further or visit opaque NBT.
    pub fn visit_children<'a>(&'a self, mut visit: impl FnMut(&'a Text)) {
        for child in &self.children {
            visit(child);
        }
        match &self.content {
            TextContent::Translatable { arguments, .. } => {
                for child in arguments {
                    visit(child);
                }
            }
            TextContent::Selector { separator, .. } | TextContent::Nbt { separator, .. } => {
                if let Some(child) = separator {
                    visit(child);
                }
            }
            TextContent::Object(
                TextObject::Player { fallback, .. } | TextObject::Atlas { fallback, .. },
            ) => {
                if let Some(child) = fallback {
                    visit(child);
                }
            }
            _ => {}
        }
        match &self.style.hover {
            Some(TextHover::Text(child)) => visit(child),
            Some(TextHover::Entity {
                name: Some(child), ..
            }) => visit(child),
            _ => {}
        }
    }
}

pub(crate) fn validate_profile(profile: &TextProfile) -> Result<(), TextFormError> {
    if !matches!(profile.metadata, Tag::Compound(_))
        || profile
            .properties
            .iter()
            .any(|p| !matches!(p.metadata, Tag::Compound(_)))
    {
        return Err(TextFormError::Malformed);
    }
    // ResolvableProfile.Partial.MAP_CODEC and ExtraCodecs.PLAYER_NAME.
    if profile
        .name
        .as_ref()
        .is_some_and(|n| n.len() > 16 || n.bytes().any(|b| b <= 32 || b >= 127))
    {
        return Err(TextFormError::Malformed);
    }
    // ExtraCodecs.PROPERTY_MAP accepts either up to 16 map keys with
    // unbounded unsigned value lists, or up to 16 signed property records.
    let unsigned = profile
        .properties
        .iter()
        .all(|p| p.signature.is_none() && matches!(&p.metadata, Tag::Compound(m) if m.is_empty()));
    if unsigned {
        let names: std::collections::BTreeSet<_> =
            profile.properties.iter().map(|p| &p.name).collect();
        if names.len() > 16 {
            return Err(TextFormError::Malformed);
        }
    } else if profile.properties.len() > 16
        || profile.properties.iter().any(|p| {
            p.name.encode_utf16().count() > 64
                || p.value.encode_utf16().count() > 32767
                || p.signature
                    .as_ref()
                    .is_some_and(|s| s.encode_utf16().count() > 1024)
        })
    {
        return Err(TextFormError::Malformed);
    }
    Ok(())
}

fn validate_payload(
    metadata: &Tag,
    style_metadata: &Tag,
    click: Option<&TextClick>,
    item: Option<&Tag>,
    profile: Option<&TextProfile>,
) -> Result<(), TextFormError> {
    if !matches!(metadata, Tag::Compound(_)) || !matches!(style_metadata, Tag::Compound(_)) {
        return Err(TextFormError::Malformed);
    }
    let mut roots = vec![metadata, style_metadata];
    if let Some(profile) = profile {
        validate_profile(profile)?;
        roots.push(&profile.metadata);
        roots.extend(profile.properties.iter().map(|p| &p.metadata));
    }
    if let Some(TextClick::ShowDialog(tag)) = click {
        roots.push(tag);
    }
    if let Some(TextClick::Custom {
        payload: Some(tag), ..
    }) = click
    {
        roots.push(tag);
    }
    roots.extend(item);
    if roots
        .into_iter()
        .any(|tag| !sunrise_core::nbt::within_depth(tag, MAX_TEXT_DEPTH))
    {
        return Err(TextFormError::TooDeep);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{Reader, WireError, Writer};
    use crate::{PAYLOAD_DECODE_BUDGET, decode_payload, encode_payload};

    fn named(name: &str) -> TextProfile {
        TextProfile {
            name: Some(name.into()),
            ..Default::default()
        }
    }

    fn encode(text: &Text) -> Result<Vec<u8>, WireError> {
        let mut out = Writer::new();
        out.put(text);
        match out.error() {
            Some(error) => Err(error),
            None => Ok(out.into_bytes()),
        }
    }

    #[test]
    fn visit_children_lends_appended_components_then_arguments_then_hover() {
        let mut root = Text::translate(
            "key",
            vec![Text::literal("arg one"), Text::literal("arg two")],
        )
        .append(Text::literal("child one").append(Text::literal("grandchild")))
        .append(Text::literal("child two"));
        root.style.hover = Some(TextHover::Text(Box::new(Text::literal("hover"))));

        let mut visited = Vec::new();
        root.visit_children(|child| {
            let TextContent::Literal(value) = &child.content else {
                panic!("expected a literal child");
            };
            visited.push(value.as_str());
        });
        assert_eq!(
            visited,
            ["child one", "child two", "arg one", "arg two", "hover"]
        );
    }

    #[test]
    fn recursive_fields_styles_and_metadata_survive_wire() {
        let mut root =
            Text::translate("key", vec![Text::literal("first"), Text::literal("second")]);
        let mut player = Text::new(TextContent::Object(TextObject::Player {
            profile: TextProfile {
                name: Some("Test".into()),
                id: Some([1, 2]),
                properties: vec![TextProfileProperty {
                    name: "textures".into(),
                    value: "data".into(),
                    signature: Some("signed".into()),
                    metadata: Tag::compound([("property-extra", Tag::Int(8))]),
                }],
                metadata: Tag::compound([(
                    "future-profile",
                    Tag::List(vec![Tag::String("kept".into())]),
                )]),
            },
            hat: false,
            fallback: Some(Box::new(Text::translate("head", vec![]))),
        }));
        player.style.hover = Some(TextHover::Text(Box::new(Text::literal("hover"))));
        let mut atlas = Text::new(TextContent::Object(TextObject::Atlas {
            atlas: "minecraft:blocks".into(),
            sprite: "test:icon".into(),
            fallback: None,
        }));
        atlas.style.hover = Some(TextHover::Entity {
            kind: "test:entity".into(),
            id: [3, 4],
            name: Some(Box::new(Text::literal("name"))),
        });
        root.children = vec![
            Text::new(TextContent::Selector {
                pattern: "@a".into(),
                separator: Some(Box::new(Text::literal(", "))),
            }),
            Text::new(TextContent::Nbt {
                path: "x".into(),
                interpret: true,
                source: TextNbtSource::Storage("test:data".into()),
                separator: Some(Box::new(Text::literal("/"))),
            }),
            player,
            atlas,
        ];
        root.style.bold = Some(false);
        root.metadata = Tag::compound([("future", Tag::Byte(7))]);
        root.style.metadata = Tag::compound([("future-style", Tag::IntArray(vec![1, 2]))]);

        let bytes = encode(&root).unwrap();
        assert_eq!(Reader::decode::<Text>(&bytes).unwrap(), root);
        let payload = encode_payload(&root, PAYLOAD_DECODE_BUDGET).unwrap();
        assert_eq!(payload, bytes);
        assert_eq!(decode_payload::<Text>(&payload).unwrap(), root);
    }

    #[test]
    fn malformed_metadata_and_wire_tags_refuse() {
        let mut bad_meta = Text::literal("root");
        bad_meta.metadata = Tag::Byte(1);
        assert_eq!(bad_meta.validate(), Err(TextFormError::Malformed));
        assert!(matches!(encode(&bad_meta), Err(WireError::Invalid(_))));

        let mut bad_style_meta = Text::literal("root");
        bad_style_meta.style.metadata = Tag::List(vec![]);
        assert_eq!(bad_style_meta.validate(), Err(TextFormError::Malformed));
        assert!(matches!(
            encode(&bad_style_meta),
            Err(WireError::Invalid(_))
        ));

        let valid = encode(&Text::literal("a")).unwrap();
        let mut non_compound_meta = valid.clone();
        let meta_offset = non_compound_meta.len() - 8;
        non_compound_meta.truncate(meta_offset);
        non_compound_meta.extend([1, 0, 0, 0, 7]); // Tag::Byte(7)
        assert!(matches!(
            Reader::decode::<Text>(&non_compound_meta),
            Err(WireError::Invalid(_))
        ));

        let mut bad_content_tag = valid;
        bad_content_tag[..4].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(
            Reader::decode::<Text>(&bad_content_tag),
            Err(WireError::Tag(99))
        );
    }

    #[test]
    fn fallback_absence_and_component_styles_remain_distinct() {
        for fallback in [
            None,
            Some(Text::literal("styled").with_style(TextStyle {
                bold: Some(false),
                ..Default::default()
            })),
            Some(Text::translate("test.head", vec![])),
        ] {
            for player in [true, false] {
                let object = if player {
                    TextObject::Player {
                        profile: named("Test"),
                        hat: true,
                        fallback: fallback.clone().map(Box::new),
                    }
                } else {
                    TextObject::Atlas {
                        atlas: "minecraft:blocks".into(),
                        sprite: "test:icon".into(),
                        fallback: fallback.clone().map(Box::new),
                    }
                };
                let text = Text::new(TextContent::Object(object));
                let bytes = encode(&text).unwrap();
                assert_eq!(Reader::decode::<Text>(&bytes).unwrap(), text);
            }
        }
    }

    #[test]
    fn profile_limits_cover_both_vanilla_property_codecs() {
        let mut profile = named("Test");
        profile.properties = (0..17)
            .map(|_| TextProfileProperty {
                name: "textures".into(),
                value: "v".into(),
                ..Default::default()
            })
            .collect();
        let text = |profile| {
            Text::new(TextContent::Object(TextObject::Player {
                profile,
                hat: true,
                fallback: None,
            }))
        };
        let valid = text(profile.clone());
        assert!(valid.validate().is_ok()); // one unsigned map key, not 17 signed entries
        let bytes = encode(&valid).unwrap();
        assert_eq!(Reader::decode::<Text>(&bytes).unwrap(), valid);

        profile.properties[0].signature = Some("signed".into());
        let signed = text(profile.clone());
        assert_eq!(signed.validate(), Err(TextFormError::Malformed));
        assert!(matches!(encode(&signed), Err(WireError::Invalid(_))));

        profile.properties.clear();
        profile.name = Some("seventeen-chars!!!".into());
        let long_name = text(profile);
        assert_eq!(long_name.validate(), Err(TextFormError::Malformed));
        assert!(matches!(encode(&long_name), Err(WireError::Invalid(_))));
    }

    fn nested_tag(depth: usize) -> Tag {
        (0..depth).fold(Tag::Byte(7), |t, _| Tag::List(vec![t]))
    }

    #[test]
    fn every_opaque_slot_obeys_the_512_container_boundary() {
        for depth in [512, 513] {
            for slot in 0..7 {
                let mut text = Text::literal("root");
                let metadata = || Tag::compound([("test:deep", nested_tag(depth - 1))]);
                match slot {
                    0 => text.metadata = metadata(),
                    1 => text.style.metadata = metadata(),
                    2 | 3 => {
                        let mut profile = named("Test");
                        if slot == 2 {
                            profile.metadata = metadata();
                        } else {
                            let property = TextProfileProperty {
                                name: "textures".into(),
                                value: "value".into(),
                                metadata: metadata(),
                                ..Default::default()
                            };
                            profile.properties.push(property);
                        }
                        text.content = TextContent::Object(TextObject::Player {
                            profile,
                            hat: true,
                            fallback: None,
                        });
                    }
                    4 => {
                        text.style.click = Some(TextClick::Custom {
                            id: "test:x".into(),
                            payload: Some(nested_tag(depth)),
                        })
                    }
                    5 => text.style.click = Some(TextClick::ShowDialog(nested_tag(depth))),
                    _ => {
                        text.style.hover = Some(TextHover::Item {
                            id: "minecraft:stone".into(),
                            count: 1,
                            components: Some(nested_tag(depth)),
                        })
                    }
                }
                if depth == 512 {
                    assert!(text.validate().is_ok(), "slot {slot}");
                    let bytes = encode(&text).unwrap();
                    assert_eq!(Reader::decode::<Text>(&bytes).unwrap(), text, "slot {slot}");
                } else {
                    assert_eq!(text.validate(), Err(TextFormError::TooDeep), "slot {slot}");
                    assert_eq!(encode(&text), Err(WireError::Depth), "slot {slot}");
                }
            }
        }
    }

    #[test]
    fn mixed_component_and_tag_depths_have_no_100_container_gap() {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let mut tree = Text::literal("leaf");
                tree.metadata = Tag::compound([("deep", nested_tag(511))]);
                tree.style.metadata = Tag::compound([("deep-style", nested_tag(511))]);
                tree.style.click = Some(TextClick::Custom {
                    id: "test:click".into(),
                    payload: Some(nested_tag(512)),
                });
                tree.style.hover = Some(TextHover::Item {
                    id: "test:item".into(),
                    count: 1,
                    components: Some(nested_tag(512)),
                });
                for _ in 1..512 {
                    tree = Text::literal("parent").append(tree);
                }
                let bytes = encode(&tree).unwrap();
                let decoded = Reader::decode::<Text>(&bytes).unwrap();
                let mut leaf = &decoded;
                for _ in 1..512 {
                    leaf = &leaf.children[0];
                }
                assert!(matches!(leaf.metadata.get("deep"), Some(Tag::List(_))));
                let too_deep = Text::literal("root").append(tree);
                assert_eq!(too_deep.validate(), Err(TextFormError::TooDeep));
                assert_eq!(encode(&too_deep), Err(WireError::Depth));
                // Drop both deep trees directly on the 256 KiB stack to exercise iterative Drop.
                drop(too_deep);
                drop(decoded);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn dropping_a_deep_text_tree_is_stack_independent() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut tree = Text::literal("leaf");
                tree.metadata = Tag::compound([("deep", nested_tag(2048))]);
                for step in 0..2048 {
                    tree = match step % 3 {
                        0 => Text::literal("c").append(tree),
                        1 => Text::literal("h").with_style(TextStyle {
                            hover: Some(TextHover::Text(Box::new(tree))),
                            ..Default::default()
                        }),
                        _ => Text::new(TextContent::Selector {
                            pattern: "@s".into(),
                            separator: Some(Box::new(tree)),
                        }),
                    };
                }
                drop(tree);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn opaque_nonfinite_and_surrogates_are_data_until_json_emission() {
        let mut text = Text::literal("data");
        text.metadata = Tag::compound([
            ("number", Tag::Float(f32::NAN.to_bits())),
            (
                "string",
                Tag::String(sunrise_core::nbt::JavaString(vec![0xd800])),
            ),
        ]);
        let bytes = encode(&text).unwrap();
        assert_eq!(Reader::decode::<Text>(&bytes).unwrap(), text);
    }
}
