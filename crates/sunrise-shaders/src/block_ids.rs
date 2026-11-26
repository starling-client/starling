//! `block.properties` (spec 4.1): `block.<id>=<block> <block> ...`, each block
//! `[namespace:]name` with optional state filters `:<property>=<v1>,<v2>`. Values in one filter
//! are alternatives; separate filters must all match. Names the game does not know are simply
//! never asked for. Block tags (`%tag`, tier E) are not read for ids yet.
//!
//! The same file's `layer.<layer>=<block> ...` lines (4.2) move block states to another terrain
//! layer ([`BlockLayers`]): their words are read by the same parser, and they also take `%tag`.

use crate::Properties;

/// One word's state filters: `(property, values)`, each matching one of its values.
type Filters = Vec<(String, Vec<String>)>;

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    id: i32,
    /// `minecraft:stone`.
    name: String,
    filters: Filters,
}

/// The ids `block.properties` gives block states.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockIds {
    rules: Vec<Rule>,
    /// Lines skipped and why.
    pub notes: Vec<String>,
}

/// A block word, `[namespace:]name[:<property>=<v1>,<v2>...]...`, as its full name and filters;
/// `None` for a word with more than two name parts.
fn block_word(word: &str) -> Option<(String, Filters)> {
    let (names, filters): (Vec<&str>, Vec<&str>) = word.split(':').partition(|p| !p.contains('='));
    let name = match names[..] {
        [name] => format!("minecraft:{name}"),
        [namespace, name] => format!("{namespace}:{name}"),
        _ => return None,
    };
    let filters = filters
        .iter()
        .filter_map(|f| f.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.split(',').map(str::to_owned).collect()))
        .collect();
    Some((name, filters))
}

/// Whether a state with `properties` passes every filter.
fn filters_match<'a>(filters: &Filters, properties: &impl Fn(&str) -> Option<&'a str>) -> bool {
    filters
        .iter()
        .all(|(k, vs)| properties(k).is_some_and(|v| vs.iter().any(|x| x == v)))
}

impl BlockIds {
    pub fn parse(properties: &Properties) -> Self {
        Self::parse_kind(properties, "block")
    }

    /// The same for `<kind>.<id>=` lines: `entity` in `entity.properties`, `item` in
    /// `item.properties` (4.1); names there carry no state filters.
    pub fn parse_kind(properties: &Properties, kind: &str) -> Self {
        let mut out = Self::default();
        let prefix = format!("{kind}.");
        for e in &properties.entries {
            let Some(id) = e.key.strip_prefix(prefix.as_str()) else {
                continue;
            };
            let Ok(id) = id.parse::<i32>() else {
                // `layer.*` (4.2) and other keys are not ids.
                continue;
            };
            for word in e.value.split_whitespace() {
                if word.starts_with('%') {
                    out.notes.push(format!(
                        "line {}: block tags are not read yet: {word}",
                        e.line
                    ));
                    continue;
                }
                let Some((name, filters)) = block_word(word) else {
                    out.notes
                        .push(format!("line {}: not a block name: {word}", e.line));
                    continue;
                };
                out.rules.push(Rule { id, name, filters });
            }
        }
        out
    }

    /// The id of block `name` (`minecraft:stone`) in the state with `properties`. The rule with
    /// the most filters that all match wins, then the first in the file (one name belongs to
    /// one id; a filtered entry is the more specific one).
    pub fn id<'a>(&self, name: &str, properties: impl Fn(&str) -> Option<&'a str>) -> Option<i32> {
        self.rules
            .iter()
            .filter(|r| r.name == name)
            .filter(|r| filters_match(&r.filters, &properties))
            .min_by_key(|r| std::cmp::Reverse(r.filters.len()))
            .map(|r| r.id)
    }

    /// Every name without state filters and its id (the first line naming it wins): what
    /// `entity.properties` and `item.properties` hold.
    pub fn plain(&self) -> std::collections::HashMap<String, i32> {
        let mut out = std::collections::HashMap::new();
        for r in self.rules.iter().filter(|r| r.filters.is_empty()) {
            out.entry(r.name.clone()).or_insert(r.id);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// A terrain layer `layer.<layer>` names (4.2). `layer.cutout_mipped` is [`RenderLayer::Cutout`]:
/// the terrain has one alpha-tested pass (the mesher's solid quads without `OPAQUE_LAYER`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderLayer {
    /// No alpha test, no blending.
    Solid,
    /// Alpha test.
    Cutout,
    /// Blending, drawn with the translucent terrain (`gbuffers_water`).
    Translucent,
}

impl RenderLayer {
    /// The layer of `layer.<name>`, or `None` for a name 4.2 doesn't list.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "solid" => Some(Self::Solid),
            "cutout" | "cutout_mipped" => Some(Self::Cutout),
            "translucent" => Some(Self::Translucent),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct LayerRule {
    layer: RenderLayer,
    /// A block's full name, or a tag's for `%tag` (`minecraft:leaves`).
    name: String,
    tag: bool,
    filters: Filters,
    /// The line and the word that wrote it, for notes.
    line: u32,
    word: String,
}

/// The `layer.*` rule a state matched (`BlockLayers::pick`): its layer, and the line and the
/// word that wrote it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerMatch<'a> {
    pub layer: RenderLayer,
    pub line: u32,
    pub word: &'a str,
}

/// The terrain layers `block.properties`' `layer.*` lines (4.2) give block states.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockLayers {
    /// In file order.
    rules: Vec<LayerRule>,
    /// Lines and words skipped and why.
    pub notes: Vec<String>,
}

impl BlockLayers {
    pub fn parse(properties: &Properties) -> Self {
        let mut out = Self::default();
        for e in &properties.entries {
            let Some(name) = e.key.strip_prefix("layer.") else {
                continue;
            };
            let Some(layer) = RenderLayer::named(name) else {
                out.notes
                    .push(format!("line {}: not a render layer: {}", e.line, e.key));
                continue;
            };
            for word in e.value.split_whitespace() {
                let (tag, block) = match word.strip_prefix('%') {
                    Some(tag) => (true, tag),
                    None => (false, word),
                };
                let Some((name, filters)) = block_word(block) else {
                    out.notes
                        .push(format!("line {}: not a block name: {word}", e.line));
                    continue;
                };
                out.rules.push(LayerRule {
                    layer,
                    name,
                    tag,
                    filters,
                    line: e.line,
                    word: word.to_owned(),
                });
            }
        }
        out
    }

    /// Each rule's index, the name it names (a block's, or a tag's for `%tag`) and whether that
    /// is a tag, in file order: what a caller indexes the rules of a block by.
    pub fn subjects(&self) -> impl Iterator<Item = (usize, &str, bool)> {
        self.rules
            .iter()
            .enumerate()
            .map(|(i, r)| (i, r.name.as_str(), r.tag))
    }

    /// Of the rules `candidates` (indices from [`Self::subjects`] whose subject names the block),
    /// the one for the state with `properties`: a rule naming the block wins over one naming its
    /// tag (4.1), then the rule with the most filters that all match, then the first in the file.
    /// `None` when none matches, and the block keeps its own layer.
    pub fn pick<'a, 'p>(
        &'a self,
        candidates: impl IntoIterator<Item = usize>,
        properties: impl Fn(&str) -> Option<&'p str>,
    ) -> Option<LayerMatch<'a>> {
        candidates
            .into_iter()
            .filter_map(|i| Some((i, self.rules.get(i)?)))
            .filter(|(_, r)| filters_match(&r.filters, &properties))
            .min_by_key(|&(i, r)| (r.tag, std::cmp::Reverse(r.filters.len()), i))
            .map(|(_, r)| LayerMatch {
                layer: r.layer,
                line: r.line,
                word: &r.word,
            })
    }

    /// [`Self::pick`] among every rule, `in_tag` telling whether block `name` is in a tag.
    pub fn layer<'a>(
        &self,
        name: &str,
        properties: impl Fn(&str) -> Option<&'a str>,
        in_tag: impl Fn(&str) -> bool,
    ) -> Option<RenderLayer> {
        let candidates = self
            .subjects()
            .filter(|&(_, subject, tag)| {
                if tag {
                    in_tag(subject)
                } else {
                    subject == name
                }
            })
            .map(|(i, ..)| i);
        self.pick(candidates, properties).map(|m| m.layer)
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Macros;

    #[test]
    fn names_states_and_precedence() {
        let text = "block.10=stone minecraft:dirt mod:thing \\\n grass_block:snowy=false\n\
                    block.11=grass_block\nblock.12=oak_leaves:persistent=true:distance=1,2 %leaves\n\
                    layer.translucent=glass\n";
        let ids = BlockIds::parse(&Properties::parse(text, &Macros::standard()));
        let none = |_: &str| None;
        assert_eq!(ids.id("minecraft:stone", none), Some(10));
        assert_eq!(ids.id("mod:thing", none), Some(10));
        assert_eq!(ids.id("minecraft:glass", none), None);
        let snowy = |s: &'static str| move |k: &str| (k == "snowy").then_some(s);
        // The filtered entry is the more specific one.
        assert_eq!(ids.id("minecraft:grass_block", snowy("false")), Some(10));
        assert_eq!(ids.id("minecraft:grass_block", snowy("true")), Some(11));
        let leaves = |p: &'static str, d: &'static str| {
            move |k: &str| match k {
                "persistent" => Some(p),
                "distance" => Some(d),
                _ => None,
            }
        };
        assert_eq!(
            ids.id("minecraft:oak_leaves", leaves("true", "2")),
            Some(12)
        );
        assert_eq!(ids.id("minecraft:oak_leaves", leaves("true", "3")), None);
        assert_eq!(ids.id("minecraft:oak_leaves", leaves("false", "1")), None);
        assert_eq!(ids.notes.len(), 1, "{:?}", ids.notes);
    }

    #[test]
    fn entity_lines_by_name() {
        let text = "entity.10=zombie minecraft:husk\nentity.11=zombie mod:thing\nblock.5=stone\n";
        let ids = BlockIds::parse_kind(&Properties::parse(text, &Macros::standard()), "entity");
        let plain = ids.plain();
        assert_eq!(
            plain["minecraft:zombie"], 10,
            "the first line naming it wins"
        );
        assert_eq!(plain["minecraft:husk"], 10);
        assert_eq!(plain["mod:thing"], 11);
        assert!(!plain.contains_key("minecraft:stone"));
    }

    /// The issue's control: one property's value moves a state, the other states of the same
    /// block keep theirs.
    #[test]
    fn layer_rules_move_named_states_and_tags() {
        let text = "block.10=oak_log\n\
                    layer.translucent=minecraft:oak_log:axis=x %logs:axis=z\n\
                    layer.cutout=birch_log:axis=x mod:glass %minecraft:leaves\n\
                    layer.cutout_mipped=dirt\n\
                    layer.solid=stained:a:b oak_leaves:persistent=true\n\
                    layer.water=stone\n\
                    layer.=stone\n";
        let layers = BlockLayers::parse(&Properties::parse(text, &Macros::standard()));
        let axis = |a: &'static str| move |k: &str| (k == "axis").then_some(a);
        let no_tag = |_: &str| false;
        let logs = |t: &str| t == "minecraft:logs";
        use RenderLayer::*;
        assert_eq!(
            layers.layer("minecraft:oak_log", axis("x"), no_tag),
            Some(Translucent)
        );
        assert_eq!(layers.layer("minecraft:oak_log", axis("y"), no_tag), None);
        assert_eq!(
            layers.layer("minecraft:birch_log", axis("x"), no_tag),
            Some(Cutout)
        );
        assert_eq!(layers.layer("minecraft:birch_log", axis("y"), no_tag), None);
        // Through `%logs`, with its filter: `birch_log` axis=z is in the tag.
        assert_eq!(
            layers.layer("minecraft:birch_log", axis("z"), logs),
            Some(Translucent)
        );
        assert_eq!(layers.layer("minecraft:birch_log", axis("y"), logs), None);
        assert_eq!(layers.layer("mod:glass", |_| None, no_tag), Some(Cutout));
        assert_eq!(
            layers.layer("minecraft:dirt", |_| None, no_tag),
            Some(Cutout)
        );
        // A rule naming the block wins over its tag, even with fewer filters.
        let leaves = |t: &str| t == "minecraft:leaves";
        let persistent = |k: &str| (k == "persistent").then_some("true");
        assert_eq!(
            layers.layer("minecraft:oak_leaves", persistent, leaves),
            Some(Solid)
        );
        assert_eq!(
            layers.layer("minecraft:oak_leaves", |_| Some("false"), leaves),
            Some(Cutout)
        );
        // Unknown layer names and a word of three name parts are refused, by name.
        assert_eq!(layers.layer("minecraft:stone", |_| None, no_tag), None);
        assert_eq!(
            layers.notes,
            [
                "line 5: not a block name: stained:a:b",
                "line 6: not a render layer: layer.water",
                "line 7: not a render layer: layer.",
            ]
        );
        let tags: Vec<_> = layers.subjects().filter(|s| s.2).map(|s| s.1).collect();
        assert_eq!(tags, ["minecraft:logs", "minecraft:leaves"]);
    }

    /// The most filters win among rules naming the block, then the first in the file.
    #[test]
    fn layer_rule_precedence() {
        let text = "layer.cutout=glass\nlayer.translucent=glass glass:a=1 glass:a=1:b=2\n\
                    layer.solid=glass:b=2:a=1\n";
        let layers = BlockLayers::parse(&Properties::parse(text, &Macros::standard()));
        let props = |a: &'static str, b: &'static str| {
            move |k: &str| match k {
                "a" => Some(a),
                "b" => Some(b),
                _ => None,
            }
        };
        let no_tag = |_: &str| false;
        use RenderLayer::*;
        assert_eq!(
            layers.layer("minecraft:glass", props("0", "0"), no_tag),
            Some(Cutout)
        );
        assert_eq!(
            layers.layer("minecraft:glass", props("1", "0"), no_tag),
            Some(Translucent)
        );
        assert_eq!(
            layers.layer("minecraft:glass", props("1", "2"), no_tag),
            Some(Translucent)
        );
        // The winning rule is named by its line and word, for a note.
        let glass = layers
            .subjects()
            .filter(|s| s.1 == "minecraft:glass")
            .map(|s| s.0);
        assert_eq!(
            layers.pick(glass, props("1", "2")),
            Some(LayerMatch {
                layer: Translucent,
                line: 2,
                word: "glass:a=1:b=2"
            })
        );
        assert!(layers.notes.is_empty());
        // A file without `layer.*` lines has no rules; its id lines are not layers.
        let ids = Properties::parse("block.5=stone\n", &Macros::standard());
        assert!(BlockLayers::parse(&ids).is_empty());
    }
}
