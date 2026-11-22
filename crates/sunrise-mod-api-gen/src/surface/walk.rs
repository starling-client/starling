//! The public items of `mod_api`, each under the path a mod writes: the facade crate's root,
//! through public modules and `pub use`s (globs included), skipping `#[doc(hidden)]`, `__`
//! names and items compiled out of a mod's default native build.
use crate::index::{Found, Index, ItemId, OutsideSeam, uses};
use mod_source::index::Binding;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use syn::{Item, Visibility};

/// The facade crate a mod depends on, and the crate its root re-exports.
const FACADE: &str = "sunrise_mod_api";
const VOCAB: &str = "mod_api";
/// The procedural macros crate whose macros the API re-exports.
pub(super) const MACROS: &str = "sunrise_mod_api_macros";

/// What a public name leads to.
#[derive(Clone, Debug)]
pub(super) enum Target {
    Item(ItemId),
    Module(usize),
    External(Vec<String>),
    /// A resolved outside definition; its canonical path need not be publicly reachable.
    OutsideItem(Vec<String>),
    Ambiguous(String),
    ExternalGlob {
        path: Vec<String>,
        written: Vec<String>,
    },
}

/// A public name of a module.
#[derive(Clone)]
struct Name {
    name: String,
    target: Target,
    hidden: bool,
}

/// The walk's result.
#[derive(Default)]
pub(super) struct Public {
    /// Each public item with its id path, below `mod_api`: the path of the module that defines
    /// it when that is public, otherwise its shortest public path.
    pub(super) items: HashMap<ItemId, Vec<String>>,
    /// Every public path of each public item, sorted.
    pub(super) paths: HashMap<ItemId, Vec<String>>,
    /// Items a mod can name only through a `#[doc(hidden)]` item or route: part of the surface
    /// only when a public signature names them.
    pub(super) hidden: HashMap<ItemId, Vec<String>>,
    /// Re-exported procedural macros: the public path and the macro's name in its crate.
    pub(super) macros: Vec<(Vec<String>, String)>,
    /// What the walk could not classify, one line each.
    pub(super) notes: Vec<String>,
}

/// Whether `attrs` hold `#[doc(hidden)]`.
pub(super) fn hidden(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        matches!(&attr.meta, syn::Meta::List(list)
            if list.path.is_ident("doc") && list.tokens.to_string() == "hidden")
    })
}

pub(super) fn attrs(item: &Item) -> &[syn::Attribute] {
    match item {
        Item::Const(i) => &i.attrs,
        Item::Enum(i) => &i.attrs,
        Item::Fn(i) => &i.attrs,
        Item::Macro(i) => &i.attrs,
        Item::Mod(i) => &i.attrs,
        Item::Static(i) => &i.attrs,
        Item::Struct(i) => &i.attrs,
        Item::Trait(i) => &i.attrs,
        Item::Type(i) => &i.attrs,
        Item::Union(i) => &i.attrs,
        Item::Use(i) => &i.attrs,
        Item::Impl(i) => &i.attrs,
        _ => &[],
    }
}

pub(super) fn defined(item: &Item) -> Option<(&Visibility, String)> {
    match item {
        Item::Const(i) => Some((&i.vis, i.ident.to_string())),
        Item::Enum(i) => Some((&i.vis, i.ident.to_string())),
        Item::Fn(i) => Some((&i.vis, i.sig.ident.to_string())),
        Item::Static(i) => Some((&i.vis, i.ident.to_string())),
        Item::Struct(i) => Some((&i.vis, i.ident.to_string())),
        Item::Trait(i) => Some((&i.vis, i.ident.to_string())),
        Item::Type(i) => Some((&i.vis, i.ident.to_string())),
        Item::Union(i) => Some((&i.vis, i.ident.to_string())),
        _ => None,
    }
}

pub(super) struct Walker<'i, 'a> {
    pub(super) index: &'i mut Index<'a>,
    names: HashMap<usize, Vec<Name>>,
    open: HashSet<usize>,
    cycle_cuts: usize,
    notes: Vec<String>,
    /// Public items compiled out only under `cfg(test)`, keyed by module and item position.
    test_only: HashSet<(usize, usize)>,
}

/// How many paths through re-exports one module is visited by.
const PATHS_PER_MODULE: usize = 16;

/// The `cfg` attributes of `attrs`, as written.
pub(super) fn cfg_text(attrs: &[syn::Attribute]) -> String {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("cfg"))
        .map(|attr| super::rust::print(quote::ToTokens::to_token_stream(&attr.meta)))
        .collect::<Vec<_>>()
        .join(", ")
}

impl<'i, 'a> Walker<'i, 'a> {
    pub(super) fn new(index: &'i mut Index<'a>) -> Self {
        Self {
            index,
            names: HashMap::new(),
            open: HashSet::new(),
            cycle_cuts: 0,
            notes: Vec::new(),
            test_only: HashSet::new(),
        }
    }

    fn module_name(&self, module: usize) -> String {
        let m = &self.index.modules[module];
        std::iter::once(m.krate.as_str())
            .chain(m.path.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("::")
    }

    /// The module a path written in `module` names, if it is one of an indexed crate.
    fn module_at(
        &mut self,
        module: usize,
        path: &[String],
        outside: &OutsideSeam,
    ) -> Option<usize> {
        outside.internal_module(self.index, module, path)
    }

    /// What a path rooted in the extern prelude names, without letting a local module shadow its
    /// first segment. API crate roots are already in the index when the public walk starts.
    fn rooted_target(
        &mut self,
        module: usize,
        path: &[String],
        outside: &OutsideSeam,
    ) -> Result<Option<Target>, String> {
        if let Some(external) = outside.external_path(self.index, module, path, true)? {
            return Ok(Some(Target::External(external)));
        }
        if let Binding::Found(Found::Module(target)) =
            self.index.resolve_type_binding(module, path, true)?
        {
            return Ok(Some(Target::Module(target)));
        }
        // This can also name a value or a macro re-exported by an indexed module.
        let Some((last, parent)) = path.split_last() else {
            return Ok(None);
        };
        if !parent.is_empty()
            && let Binding::Found(Found::Module(parent)) =
                self.index.resolve_type_binding(module, parent, true)?
        {
            if let Some(name) = self
                .names(parent, outside)?
                .into_iter()
                .find(|name| name.name == *last)
            {
                return Ok(Some(name.target));
            }
            if let Ok(Found::Item(id)) = self.index.resolve_rooted_path(module, path) {
                let target = outside
                    .external_item_path(self.index, id)
                    .map_or_else(|| Target::Item(id), Target::OutsideItem);
                return Ok(Some(target));
            }
        }
        Ok(None)
    }

    /// What `path`, written in a `pub use` of `module`, leads to.
    fn target(
        &mut self,
        module: usize,
        path: &[String],
        outside: &OutsideSeam,
        rooted: bool,
    ) -> Result<Target, String> {
        if rooted {
            if let Some(target) = self.rooted_target(module, path, outside)? {
                return Ok(target);
            }
            return Ok(Target::External(path.to_vec()));
        }
        if let Some(target) = self.module_at(module, path, outside) {
            return Ok(Target::Module(target));
        }
        match outside.external_path(self.index, module, path, false) {
            Ok(Some(path)) => return Ok(Target::External(path)),
            Err(error) => return Ok(Target::Ambiguous(error)),
            Ok(None) => {}
        }
        let (last, parent) = path.split_last().ok_or("an empty path")?;
        if parent.is_empty() {
            // A name the module does not define or import is an outside crate (`serde_json`).
            return Ok(match self.index.resolve(module, last) {
                Ok(Found::Item(id)) => Target::Item(id),
                Ok(Found::Module(target)) => Target::Module(target),
                Ok(Found::External { path, .. }) => Target::External(path),
                Err(_) => Target::External(path.to_vec()),
            });
        }
        match self.module_at(module, parent, outside) {
            Some(parent) => self
                .names(parent, outside)?
                .into_iter()
                .find(|name| &name.name == last)
                .map(|name| name.target)
                .ok_or_else(|| {
                    format!(
                        "{last} is no public item of {} (a macro may make it)",
                        self.module_name(parent)
                    )
                }),
            None => Ok(Target::External(path.to_vec())),
        }
    }

    /// The public names of `module`: its public items and `pub use`s, then its globs' names.
    fn names(&mut self, module: usize, outside: &OutsideSeam) -> Result<Vec<Name>, String> {
        if let Some(names) = self.names.get(&module) {
            return Ok(names.clone());
        }
        if !self.open.insert(module) {
            self.cycle_cuts += 1;
            return Ok(Vec::new());
        }
        let cycle_cuts = self.cycle_cuts;
        let items = self.index.modules[module].items.clone();
        let mut own: Vec<Name> = Vec::new();
        let mut globbed: Vec<Name> = Vec::new();
        for (position, item) in items.iter().enumerate() {
            let attrs = attrs(item);
            if !self.index.default_build(module, attrs) {
                let public = match item {
                    Item::Use(i) => matches!(i.vis, Visibility::Public(_)),
                    Item::Mod(i) => matches!(i.vis, Visibility::Public(_)),
                    item => {
                        defined(item).is_some_and(|(vis, _)| matches!(vis, Visibility::Public(_)))
                    }
                };
                if public {
                    let gate = cfg_text(attrs);
                    if gate.contains("test")
                        && !gate.contains("feature")
                        && !gate.contains("target")
                    {
                        self.test_only.insert((module, position));
                    } else {
                        let what = match item {
                            Item::Mod(child) => format!("mod {}", child.ident),
                            Item::Use(item) => format!(
                                "use {}",
                                super::rust::print(quote::ToTokens::to_token_stream(&item.tree))
                            ),
                            item => defined(item).map_or_else(String::new, |(_, name)| name),
                        };
                        self.notes.push(format!(
                            "{}::{what}: compiled out of a default native build ({gate}), not recorded",
                            self.module_name(module)
                        ));
                    }
                }
                continue;
            }
            let hidden = hidden(attrs);
            match item {
                Item::Use(item) if matches!(item.vis, Visibility::Public(_)) => {
                    let mut found = Vec::new();
                    uses(&item.tree, &mut Vec::new(), &mut |path, alias| {
                        found.push((path, alias, item.leading_colon.is_some()))
                    });
                    for (path, alias, rooted) in found {
                        match alias {
                            Some(alias) if alias.starts_with("__") => {}
                            Some(alias) => match self.target(module, &path, outside, rooted) {
                                Ok(target) => own.push(Name {
                                    name: alias,
                                    target,
                                    hidden,
                                }),
                                Err(_) if hidden => {}
                                Err(why) => self.notes.push(format!(
                                    "pub use {} in {}: {why}",
                                    path.join("::"),
                                    self.module_name(module)
                                )),
                            },
                            None => {
                                if rooted {
                                    match self.rooted_target(module, &path, outside)? {
                                        Some(Target::Module(target)) => {
                                            for mut name in self.names(target, outside)? {
                                                name.hidden |= hidden;
                                                globbed.push(name);
                                            }
                                        }
                                        Some(Target::External(external)) => {
                                            globbed.push(Name {
                                                name: String::new(),
                                                target: Target::ExternalGlob {
                                                    path: external,
                                                    written: path.clone(),
                                                },
                                                hidden,
                                            });
                                        }
                                        _ => self.notes.push(format!(
                                            "pub use {}::* in {}: not a module of the API's crates",
                                            path.join("::"),
                                            self.module_name(module)
                                        )),
                                    }
                                } else if let Some(target) = self.module_at(module, &path, outside)
                                {
                                    for mut name in self.names(target, outside)? {
                                        name.hidden |= hidden;
                                        globbed.push(name);
                                    }
                                } else if let Some(external) = outside
                                    .external_path(self.index, module, &path, rooted)
                                    .map_err(|error| {
                                        format!("{}::*: {error}", self.module_name(module))
                                    })?
                                {
                                    globbed.push(Name {
                                        name: String::new(),
                                        target: Target::ExternalGlob {
                                            path: external,
                                            written: path.clone(),
                                        },
                                        hidden,
                                    });
                                } else {
                                    self.notes.push(format!(
                                        "pub use {}::* in {}: not a module of the API's crates",
                                        path.join("::"),
                                        self.module_name(module)
                                    ));
                                }
                            }
                        }
                    }
                }
                Item::Mod(child) if matches!(child.vis, Visibility::Public(_)) => {
                    let krate = self.index.modules[module].krate.clone();
                    let mut path = self.index.modules[module].path.clone();
                    path.push(child.ident.to_string());
                    let target = self.index.module(&krate, &path)?;
                    own.push(Name {
                        name: child.ident.to_string(),
                        target: Target::Module(target),
                        hidden,
                    });
                }
                item => {
                    if let Some((vis, name)) = defined(item)
                        && matches!(vis, Visibility::Public(_))
                    {
                        own.push(Name {
                            name,
                            target: Target::Item(ItemId {
                                module,
                                item: position,
                            }),
                            hidden,
                        });
                    }
                }
            }
        }
        for name in globbed {
            // An external glob has no name until `public()` expands it. Keep each route so a
            // hidden glob cannot shadow a later visible glob (or vice versa).
            if matches!(&name.target, Target::ExternalGlob { .. })
                || !own.iter().any(|own| own.name == name.name)
            {
                own.push(name);
            }
        }
        self.open.remove(&module);
        // A cycle cut is only valid along this route. Another caller may reach the names
        // missing from this expansion when it starts from a different module.
        if self.cycle_cuts == cycle_cuts {
            self.names.insert(module, own.clone());
        }
        Ok(own)
    }

    /// Every public item from the facade's root, breadth first, with every public path to it
    /// (each module through at most [`PATHS_PER_MODULE`] paths, never through itself). Names
    /// starting `__` are never followed.
    pub(super) fn public(&mut self, outside: &OutsideSeam) -> Result<Public, String> {
        let root = self.index.module(FACADE, &[])?;
        let mut public = Public::default();
        let mut all: HashMap<ItemId, BTreeSet<Vec<String>>> = HashMap::new();
        let mut queue = VecDeque::from([(root, Vec::<String>::new(), false, vec![root])]);
        let mut visits: HashMap<(usize, bool), usize> = HashMap::new();
        let mut macros: BTreeMap<Vec<String>, String> = BTreeMap::new();
        let mut hidden_items: HashSet<ItemId> = HashSet::new();
        while let Some((module, prefix, behind, ancestors)) = queue.pop_front() {
            let mut names = self.names(module, outside)?;
            names.sort_by(|a, b| a.name.cmp(&b.name));
            for name in names {
                let behind = behind || name.hidden;
                if let Target::ExternalGlob { path, written } = &name.target {
                    if behind {
                        public.notes.push(format!(
                            "pub use {}::* in {}: not a module of the API's crates",
                            written.join("::"),
                            self.module_name(module)
                        ));
                    } else {
                        let owner = if prefix.is_empty() {
                            format!("{VOCAB}::*")
                        } else {
                            format!("{VOCAB}::{}::*", prefix.join("::"))
                        };
                        let mut external = path.clone();
                        external.push("*".into());
                        return Err(OutsideSeam::error(&owner, &external));
                    }
                    continue;
                }
                if name.name.starts_with("__") {
                    continue;
                }
                let mut path = prefix.clone();
                path.push(name.name.clone());
                match name.target {
                    Target::Module(target) => {
                        let count = visits.entry((target, behind)).or_default();
                        if !ancestors.contains(&target) && *count < PATHS_PER_MODULE {
                            *count += 1;
                            let mut next = ancestors.clone();
                            next.push(target);
                            queue.push_back((target, path, behind, next));
                        }
                    }
                    Target::Item(id) => {
                        if behind || hidden(attrs(self.index.item(id))) {
                            hidden_items.insert(id);
                            let better = |old: &Vec<String>| (path.len(), &path) < (old.len(), old);
                            match public.hidden.get(&id) {
                                Some(old) if !better(old) => {}
                                _ => {
                                    public.hidden.insert(id, path);
                                }
                            }
                        } else {
                            all.entry(id).or_default().insert(path);
                        }
                    }
                    Target::External(_) | Target::OutsideItem(_) | Target::Ambiguous(_)
                        if behind => {}
                    Target::OutsideItem(external) => {
                        return Err(OutsideSeam::error(
                            &format!("mod_api::{}", path.join("::")),
                            &external,
                        ));
                    }
                    Target::Ambiguous(error) => {
                        return Err(format!("mod_api::{}: {error}", path.join("::")));
                    }
                    Target::External(external) => {
                        if external.first().map(String::as_str) == Some(MACROS)
                            && external.len() == 2
                        {
                            macros.entry(path).or_insert_with(|| external[1].clone());
                        } else if let Some(external) =
                            outside.external_path(self.index, module, &external, true)?
                        {
                            return Err(OutsideSeam::error(
                                &format!("mod_api::{}", path.join("::")),
                                &external,
                            ));
                        } else {
                            public.notes.push(format!(
                                "{} re-exports {}, outside the API's crates",
                                path.join("::"),
                                external.join("::")
                            ));
                        }
                    }
                    Target::ExternalGlob { .. } => unreachable!("handled before building a path"),
                }
            }
        }
        let mut aliased = 0;
        for (id, mut paths) in all {
            // Decided from the definition itself, not from the routes the walk took.
            let definition = self
                .definition_path(id)
                .filter(|_| self.publicly_defined(id));
            if let Some(path) = &definition {
                paths.insert(path.clone());
            }
            let chosen = match definition {
                Some(path) => path,
                None => {
                    aliased += 1;
                    paths
                        .iter()
                        .min_by(|a, b| (a.len(), *a).cmp(&(b.len(), *b)))
                        .cloned()
                        .expect("an item reached has a path")
                }
            };
            public.items.insert(id, chosen);
            public
                .paths
                .insert(id, paths.into_iter().map(|p| p.join("::")).collect());
        }
        let shown = &public.items;
        public.hidden.retain(|id, _| !shown.contains_key(id));
        hidden_items.retain(|id| !shown.contains_key(id));
        public.notes.push(format!(
            "{aliased} public items are not public at their definition path: their id is their shortest public path, which a shorter re-export added later would change"
        ));
        public.notes.push(format!(
            "{} items are reachable only through #[doc(hidden)] items or modules: recorded only where a recorded signature names them",
            hidden_items.len()
        ));
        if !self.test_only.is_empty() {
            public.notes.push(format!(
                "{} public items compiled only under cfg(test), not recorded",
                self.test_only.len()
            ));
        }
        public.macros = macros.into_iter().collect();
        public.notes.append(&mut self.notes);
        Ok(public)
    }

    /// Whether the module that defines `id` is public all the way from its crate's root: every
    /// module on the way a compiled, public, not hidden `mod`.
    fn publicly_defined(&mut self, id: ItemId) -> bool {
        let krate = self.index.modules[id.module].krate.clone();
        let path = self.index.modules[id.module].path.clone();
        for depth in 0..path.len() {
            let Ok(parent) = self.index.module(&krate, &path[..depth]) else {
                return false;
            };
            let public = self.index.modules[parent].items.iter().any(|item| {
                matches!(item, Item::Mod(child)
                    if child.ident == path[depth].as_str()
                        && matches!(child.vis, Visibility::Public(_))
                        && self.index.default_build(parent, &child.attrs)
                        && !hidden(&child.attrs))
            });
            if !public {
                return false;
            }
        }
        true
    }

    /// Where `id` is defined, as a path below `mod_api`, when it is defined in the API's crates.
    fn definition_path(&self, id: ItemId) -> Option<Vec<String>> {
        let module = &self.index.modules[id.module];
        if module.krate != VOCAB && module.krate != FACADE {
            return None;
        }
        let (_, name) = defined(self.index.item(id))?;
        let mut path = module.path.clone();
        path.push(name);
        Some(path)
    }

    /// Every module of `krate` a default native build compiles, private ones included.
    pub(super) fn all_modules(&mut self, krate: &str) -> Result<Vec<usize>, String> {
        let root = self.index.module(krate, &[])?;
        let mut out = vec![root];
        let mut next = 0;
        while next < out.len() {
            let module = out[next];
            next += 1;
            let children: Vec<String> = self.index.modules[module]
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Mod(child) if self.index.default_build(module, &child.attrs) => {
                        Some(child.ident.to_string())
                    }
                    _ => None,
                })
                .collect();
            for child in children {
                let mut path = self.index.modules[module].path.clone();
                path.push(child);
                match self.index.module(krate, &path) {
                    Ok(id) => out.push(id),
                    Err(why) => self
                        .notes
                        .push(format!("module {krate}::{}: {why}", path.join("::"))),
                }
            }
        }
        Ok(out)
    }

    pub(super) fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }
}
