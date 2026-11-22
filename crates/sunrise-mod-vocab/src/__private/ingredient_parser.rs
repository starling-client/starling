use crate::item::crafting::{IngredientRule, Shape};
use serde_json::Value;
use std::collections::HashSet;

/// The kind an ingredient object names: `neoforge:ingredient_type` in
/// current NeoForge, `type` in older packs.
fn ingredient_kind(o: &serde_json::Map<String, Value>) -> Option<&str> {
    o.get("neoforge:ingredient_type")
        .or_else(|| o.get("type"))?
        .as_str()
}

/// A mod's own kind: a namespaced type that is not minecraft's.
fn is_custom(kind: &str) -> bool {
    kind.contains(':') && !kind.starts_with("minecraft:")
}

/// Keep first-seen order, drop the rest.
// hidden: a game-side helper, not part of the mod surface
#[doc(hidden)]
pub fn dedup_keep_order(items: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    items.retain(|i| seen.insert(i.clone()));
}
// no wire: private parser working state includes engine NBT and unordered lookup sets; not an API transport value.
/// The walk both ingredient kinds share: a string (an id or `#tag`, as
/// `tags` resolves it), a list of them, or NeoForge's built-in kinds with
/// their children walked the same way, each answered by the flat list of
/// ids it matches in the order the sources named them. A kind outside the
/// shapes is answered by the wrapper's `custom` as the walk meets it, so a
/// shape sees its ids like any child's. A missing answer fails the parse.
/// Existing empty tags and custom answers retain their predicate but cannot
/// be placed.
#[derive(Clone, Debug)]
// hidden: shared parser output for the game-owned ingredient adapter, not a mod-facing recipe.
#[doc(hidden)]
#[non_exhaustive]
pub struct WalkedIngredient<A> {
    /// The ids it accepts, however the source shape named them.
    pub set: HashSet<String>,
    /// The same ids in the sources' order (`HolderSet`).
    pub order: Vec<String>,
    /// Compound, intersection, difference or custom display children.
    pub children: Option<(Shape, Vec<Self>)>,
    /// The wrapper's payload for a kind outside the standard shapes.
    pub custom: Option<A>,
    /// Component restrictions, independent of the enumerable item list.
    pub rule: Option<IngredientRule>,
    /// Slot display override, independent of matching.
    pub display: Option<sunrise_core::nbt::Compound>,
}
// no wire: borrowed parser or registration callbacks execute only in their owning process.
/// Answers the original JSON and kind with placement ids and a custom
/// payload, or none to fail the file.
// hidden: the game-owned ingredient adapter supplies custom ids while walking JSON.
#[doc(hidden)]
pub type CustomIds<'a, A> = &'a dyn Fn(&Value, &str) -> Option<(Vec<String>, A)>;
// no wire: borrowed parser or registration callbacks execute only in their owning process.
/// Game-owned JSON-to-NBT boundary for the hidden recipe parser.
// hidden: supplied by the game's ingredient adapter to the parser, not a mod-facing type.
#[doc(hidden)]
pub type IngredientData<'a> =
    &'a dyn Fn(&serde_json::Map<String, Value>) -> Option<sunrise_core::nbt::Compound>;
impl<A> WalkedIngredient<A> {
    /// Only component-independent ingredients can use item-list matching.
    pub fn is_simple(&self) -> bool {
        self.rule.is_none()
            && self.custom.is_none()
            && self
                .children
                .as_ref()
                .is_none_or(|(_, kids)| kids.iter().all(Self::is_simple))
    }

    /// The walk of `v`: the shapes' flat lists in the sources' order, their
    /// children kept for matching a concrete stack.
    pub fn walk(
        v: &Value,
        tags: &std::collections::HashMap<String, Vec<String>>,
        block_tags: &dyn Fn(&str) -> Option<Vec<String>>,
        custom: CustomIds<'_, A>,
        convert: IngredientData<'_>,
    ) -> Option<Self> {
        Self::walk_resolved_with_components(v, tags, block_tags, custom, convert, convert, &|id| {
            Some(id.to_owned())
        })
    }

    /// Walk `v` with separate converters for `neoforge:components` patches and
    /// `neoforge:custom_display` slot displays so component validation happens
    /// during the single decode pass.
    // hidden: game-owned single-pass ingredient decoder boundary.
    #[doc(hidden)]
    pub fn walk_with_components(
        v: &Value,
        tags: &std::collections::HashMap<String, Vec<String>>,
        block_tags: &dyn Fn(&str) -> Option<Vec<String>>,
        custom: CustomIds<'_, A>,
        convert_components: IngredientData<'_>,
        convert_display: IngredientData<'_>,
    ) -> Option<Self> {
        Self::walk_resolved_with_components(
            v,
            tags,
            block_tags,
            custom,
            convert_components,
            convert_display,
            &|id| Some(id.to_owned()),
        )
    }

    // hidden: the game-owned adapter resolves ids using its own load-time registry.
    #[doc(hidden)]
    pub fn walk_resolved(
        v: &Value,
        tags: &std::collections::HashMap<String, Vec<String>>,
        block_tags: &dyn Fn(&str) -> Option<Vec<String>>,
        custom: CustomIds<'_, A>,
        convert: IngredientData<'_>,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Self> {
        Self::walk_resolved_with_components(v, tags, block_tags, custom, convert, convert, resolve)
    }

    fn walk_resolved_with_components(
        v: &Value,
        tags: &std::collections::HashMap<String, Vec<String>>,
        block_tags: &dyn Fn(&str) -> Option<Vec<String>>,
        custom: CustomIds<'_, A>,
        convert_components: IngredientData<'_>,
        convert_display: IngredientData<'_>,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Self> {
        let mut list: Vec<String> = Vec::new();
        let mut valid = true;
        let mut named = false;
        let mut add = |s: &str| {
            named = true;
            let more: Vec<String> = match s.strip_prefix('#') {
                Some(tag) => match tags.get(tag) {
                    Some(items) => items.clone(),
                    None => {
                        valid = false;
                        return;
                    }
                },
                // `Ingredient.NON_AIR_HOLDER_SET_CODEC`: a directly named id is
                // never air, or the file fails; a tag's members aren't checked,
                // as vanilla checks only the direct list (#1273).
                None if s == "minecraft:air" || s == "air" => {
                    valid = false;
                    return;
                }
                None => vec![s.to_owned()],
            };
            for i in more {
                match resolve(&i) {
                    Some(i) if !list.contains(&i) => list.push(i),
                    Some(_) => {}
                    None => valid = false,
                }
            }
        };
        match v {
            Value::String(s) => add(s),
            // `HolderSet`'s list: every entry is an id, so any other value
            // fails the file rather than being dropped (#1273).
            Value::Array(a) => {
                for entry in a {
                    add(entry.as_str()?);
                }
            }
            Value::Object(o) => {
                return Self::walk_object(
                    v,
                    o,
                    tags,
                    block_tags,
                    custom,
                    convert_components,
                    convert_display,
                    resolve,
                );
            }
            _ => return None,
        }
        (valid && named).then(|| Self::flat(list))
    }
    /// A resolved plain ingredient, including an existing empty tag.
    fn flat(list: Vec<String>) -> Self {
        Self {
            set: list.iter().cloned().collect(),
            order: list,
            children: None,
            custom: None,
            rule: None,
            display: None,
        }
    }
    fn allows_empty(&self) -> bool {
        self.custom.is_some()
            || (self.order.is_empty() && self.children.is_none())
            || self
                .children
                .as_ref()
                .is_some_and(|(_, children)| children.iter().any(Self::allows_empty))
    }

    /// NeoForge's ingredient types (`NeoForgeRegistries.Keys.INGREDIENT_TYPES`),
    /// the standard shapes mods' recipes carry, each answered by the flat id
    /// list it matches, with the children kept for matching a concrete
    /// stack. A kind outside these is the wrapper's `custom`; one of
    /// `minecraft:`'s the wrapper does not know still fails the file.
    fn walk_object(
        v: &Value,
        o: &serde_json::Map<String, Value>,
        tags: &std::collections::HashMap<String, Vec<String>>,
        block_tags: &dyn Fn(&str) -> Option<Vec<String>>,
        custom: CustomIds<'_, A>,
        convert_components: IngredientData<'_>,
        convert_display: IngredientData<'_>,
        resolve: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Self> {
        // Current NeoForge marks a custom ingredient with
        // `neoforge:ingredient_type`; `type` is the older packs' key.
        let kind = ingredient_kind(o)?;
        let children = || {
            o.get("children")
                .or_else(|| o.get("ingredients"))
                .and_then(Value::as_array)
        };
        // The shapes' walked children, for matching at craft time: every
        // arm either fills it or returns.
        let nested;
        let kept = match kind {
            // `CompoundIngredient`: any of its children matching the stack
            // is enough — the union of their ids.
            "neoforge:compound" => {
                let subs: Vec<Self> = children()?
                    .iter()
                    .map(|s| {
                        Self::walk_resolved_with_components(
                            s,
                            tags,
                            block_tags,
                            custom,
                            convert_components,
                            convert_display,
                            resolve,
                        )
                    })
                    .collect::<Option<_>>()?;
                let mut seen = Vec::new();
                for sub in &subs {
                    for id in &sub.order {
                        if !seen.contains(id) {
                            seen.push(id.clone());
                        }
                    }
                }
                nested = Some((Shape::Compound, subs));
                seen
            }
            // `IntersectionIngredient`: every child must match the same
            // stack — the ids all of them name.
            "neoforge:intersection" => {
                let subs: Vec<Self> = children()?
                    .iter()
                    .map(|s| {
                        Self::walk_resolved_with_components(
                            s,
                            tags,
                            block_tags,
                            custom,
                            convert_components,
                            convert_display,
                            resolve,
                        )
                    })
                    .collect::<Option<_>>()?;
                let kept = subs
                    .first()?
                    .order
                    .iter()
                    .filter(|i| subs[1..].iter().all(|s| s.set.contains(*i)))
                    .cloned()
                    .collect();
                nested = Some((Shape::Intersection, subs));
                kept
            }
            // The base's ids without the subtracted's.
            "neoforge:difference" => {
                let base = Self::walk_resolved_with_components(
                    o.get("base")?,
                    tags,
                    block_tags,
                    custom,
                    convert_components,
                    convert_display,
                    resolve,
                )?;
                let gone = Self::walk_resolved_with_components(
                    o.get("subtracted")?,
                    tags,
                    block_tags,
                    custom,
                    convert_components,
                    convert_display,
                    resolve,
                )?;
                let kept = base
                    .order
                    .iter()
                    .filter(|i| !gone.is_simple() || !gone.set.contains(*i))
                    .cloned()
                    .collect();
                nested = Some((Shape::Difference, vec![base, gone]));
                kept
            }
            "neoforge:block_tag" => {
                let tag = o.get("tag")?.as_str()?;
                if tag.contains('#') {
                    return None;
                }
                let mut list = block_tags(tag)?;
                dedup_keep_order(&mut list);
                return Some(Self::flat(list));
            }
            "neoforge:components" => {
                if !matches!(o.get("items")?, Value::String(_) | Value::Array(_)) {
                    return None;
                }
                let mut base = Self::walk_resolved_with_components(
                    o.get("items")?,
                    tags,
                    block_tags,
                    custom,
                    convert_components,
                    convert_display,
                    resolve,
                )?;
                let mut patch = convert_components(o.get("components")?.as_object()?)?;
                let mut removed = Vec::new();
                patch.retain(|key, _| {
                    if let Some(id) = key.to_string_lossy().strip_prefix('!') {
                        removed.push(id.to_owned());
                        false
                    } else {
                        true
                    }
                });
                let strict = match o.get("strict") {
                    Some(v) => v.as_bool()?,
                    None => false,
                };
                base.rule = Some(IngredientRule::Components {
                    patch,
                    removed,
                    strict,
                });
                return Some(base);
            }
            "neoforge:custom_display" => {
                let base = Self::walk_resolved_with_components(
                    o.get("base")?,
                    tags,
                    block_tags,
                    custom,
                    convert_components,
                    convert_display,
                    resolve,
                )?;
                return Some(Self {
                    set: base.set.clone(),
                    order: base.order.clone(),
                    children: Some((Shape::CustomDisplay, vec![base])),
                    custom: None,
                    rule: None,
                    display: Some(convert_display(o.get("display")?.as_object()?)?),
                });
            }
            // The wrapper defines both the candidate ids and the custom payload.
            k if is_custom(k) => {
                let (list, answer) = custom(v, k)?;
                return Some(Self {
                    set: list.iter().cloned().collect(),
                    order: list,
                    children: None,
                    custom: Some(answer),
                    rule: None,
                    display: None,
                });
            }
            _ => return None,
        };
        (!kept.is_empty()
            || nested
                .as_ref()
                .is_some_and(|(_, children)| children.iter().any(Self::allows_empty)))
        .then(|| Self {
            set: kept.iter().cloned().collect(),
            order: kept,
            children: nested,
            custom: None,
            rule: None,
            display: None,
        })
    }
}

#[cfg(test)]
#[path = "ingredient_parser_tests.rs"]
mod tests;
