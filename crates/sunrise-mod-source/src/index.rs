//! The Rust items a set of crates' signatures name, found as rustc finds them: through each
//! module's items, `use`s and glob imports, into the crates `sunrise-mod-api` builds on.
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};
use syn::{Attribute, Item, Meta, UseTree, Visibility};

/// The first of `values` this tool's own target has, as `cfg!` saw it when the tool was built.
macro_rules! built_for {
    ($key:ident: $($value:literal),+) => {
        [$((cfg!($key = $value), $value)),+]
            .into_iter()
            .find_map(|(active, value)| active.then_some(value))
    };
}

/// A target `cfg` value: Cargo's, when a build script runs this, else the target this tool was
/// built for. A key the tool can't know about itself (`target_feature`) is unknown.
fn target_cfg(name: &str) -> Option<String> {
    let environment = format!("CARGO_CFG_{}", name.to_ascii_uppercase());
    if let Ok(value) = std::env::var(environment) {
        return Some(value);
    }
    let value = match name {
        "target_os" => std::env::consts::OS,
        "target_arch" => std::env::consts::ARCH,
        "target_family" => std::env::consts::FAMILY,
        "target_env" => {
            built_for!(target_env: "gnu", "msvc", "musl", "sgx", "newlib", "uclibc", "ohos")
                .unwrap_or("")
        }
        "target_vendor" => built_for!(target_vendor: "unknown", "pc", "apple", "fortanix")?,
        "target_has_atomic" => {
            return Some(
                [
                    (cfg!(target_has_atomic = "8"), "8"),
                    (cfg!(target_has_atomic = "16"), "16"),
                    (cfg!(target_has_atomic = "32"), "32"),
                    (cfg!(target_has_atomic = "64"), "64"),
                    (cfg!(target_has_atomic = "128"), "128"),
                    (cfg!(target_has_atomic = "ptr"), "ptr"),
                ]
                .into_iter()
                .filter_map(|(active, width)| active.then_some(width))
                .collect::<Vec<_>>()
                .join(","),
            );
        }
        "target_pointer_width" => return Some(usize::BITS.to_string()),
        "target_endian" if cfg!(target_endian = "little") => "little",
        "target_endian" if cfg!(target_endian = "big") => "big",
        _ => return None,
    };
    Some(value.to_owned())
}

/// The target triple this tool was built for, among the platforms Sunrise ships tools for.
const BUILT_FOR: Option<&str> = if cfg!(all(
    target_arch = "x86_64",
    target_os = "linux",
    target_env = "gnu"
)) {
    Some("x86_64-unknown-linux-gnu")
} else if cfg!(all(
    target_arch = "aarch64",
    target_os = "linux",
    target_env = "gnu"
)) {
    Some("aarch64-unknown-linux-gnu")
} else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
    Some("x86_64-apple-darwin")
} else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
    Some("aarch64-apple-darwin")
} else if cfg!(all(
    target_arch = "x86_64",
    target_os = "windows",
    target_env = "msvc"
)) {
    Some("x86_64-pc-windows-msvc")
} else {
    None
};

fn target_flag(name: &str, fallback: bool) -> bool {
    std::env::var_os(format!("CARGO_CFG_{}", name.to_ascii_uppercase())).is_some() || fallback
}

/// Whether a cfg predicate holds: `None` when it turns on a target value this tool can't know
/// about itself (`target_feature`), so an item under it is kept whichever way a `not` goes.
fn cfg_active(meta: &Meta, features: Option<&HashSet<String>>) -> Option<bool> {
    Some(match meta {
        Meta::Path(path) => {
            let Some(name) = path.get_ident().map(ToString::to_string) else {
                return Some(true);
            };
            match name.as_str() {
                "test" => false,
                "unix" | "windows" => target_cfg("target_family")?
                    .split(',')
                    .any(|item| item == name),
                "debug_assertions" => target_flag("debug_assertions", cfg!(debug_assertions)),
                "target_thread_local" => target_flag(
                    "target_thread_local",
                    target_cfg("target_family").is_some_and(|family| {
                        family
                            .split(',')
                            .any(|name| name == "unix" || name == "windows")
                    }),
                ),
                _ => true,
            }
        }
        Meta::NameValue(pair) => {
            let Some(name) = pair.path.get_ident().map(ToString::to_string) else {
                return Some(true);
            };
            if name != "feature" && !name.starts_with("target_") {
                return Some(true);
            }
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(expected),
                ..
            }) = &pair.value
            else {
                return Some(true);
            };
            if name == "feature" {
                return Some(features.is_some_and(|features| features.contains(&expected.value())));
            }
            target_cfg(&name)?
                .split(',')
                .any(|item| item.trim() == expected.value())
        }
        Meta::List(list) => {
            let Ok(inner) = list.parse_args_with(
                syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
            ) else {
                return Some(true);
            };
            let each: Vec<_> = inner
                .iter()
                .map(|meta| cfg_active(meta, features))
                .collect();
            // An unknown operand leaves the answer unknown unless a known one decides it.
            let decided = |by: bool| {
                if each.contains(&Some(by)) {
                    Some(by)
                } else if each.contains(&None) {
                    None
                } else {
                    Some(!by)
                }
            };
            if list.path.is_ident("any") {
                decided(true)?
            } else if list.path.is_ident("all") {
                decided(false)?
            } else if list.path.is_ident("not") {
                !decided(false)?
            } else {
                true
            }
        }
    })
}

fn attribute_is_active(meta: &Meta, features: Option<&HashSet<String>>) -> bool {
    match meta {
        Meta::List(list) if list.path.is_ident("cfg") => list
            .parse_args::<Meta>()
            .map(|condition| cfg_active(&condition, features) != Some(false))
            .unwrap_or(true),
        Meta::List(list) if list.path.is_ident("cfg_attr") => {
            let Ok(args) = list.parse_args_with(
                syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
            ) else {
                return true;
            };
            let Some(condition) = args.first() else {
                return true;
            };
            cfg_active(condition, features) != Some(true)
                || args
                    .iter()
                    .skip(1)
                    .all(|meta| attribute_is_active(meta, features))
        }
        _ => true,
    }
}

/// Whether a Cargo target dependency table applies to the current target.
pub fn target_dependency_active(selector: &str) -> Result<bool, String> {
    if let Some(condition) = selector
        .strip_prefix("cfg(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let meta: Meta = syn::parse_str(condition)
            .map_err(|error| format!("invalid Cargo target selector `{selector}`: {error}"))?;
        return Ok(cfg_active(&meta, None) != Some(false));
    }
    Ok(match std::env::var("TARGET") {
        Ok(target) => selector == target,
        Err(_) => BUILT_FOR == Some(selector),
    })
}

/// One module: its crate, its path below the crate root, and its items.
pub struct Module {
    pub krate: String,
    pub path: Vec<String>,
    pub items: Vec<Item>,
}

/// A named item a signature refers to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ItemId {
    pub module: usize,
    pub item: usize,
}

/// Where a name leads.
#[derive(Clone, PartialEq, Eq)]
pub enum Found {
    Item(ItemId),
    Module(usize),
    /// A path outside the indexed crates, with the scope and root of the original spelling.
    External {
        module: usize,
        path: Vec<String>,
        rooted: bool,
    },
}

/// The outcome of looking up a name without picking an arbitrary glob binding.
#[derive(Clone)]
pub enum Binding {
    Missing,
    /// A definition or explicit import shadows this module's globs, but is not visible here.
    Hidden,
    Found(Found),
    Ambiguous(String),
}

impl Binding {
    fn require(self, index: &Index<'_>, module: usize, name: &str) -> Result<Found, String> {
        match self {
            Self::Found(found) => Ok(found),
            Self::Ambiguous(error) => Err(error),
            Self::Missing | Self::Hidden => {
                let m = &index.modules[module];
                Err(format!(
                    "{name} not found in {}::{}",
                    m.krate,
                    m.path.join("::")
                ))
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Namespace {
    Any,
    Type,
}

/// Visibility must hold for every module holding a glob on the route. A restricted binding
/// is visible to all holders iff it is visible to their common ancestor. Holders in different
/// crates have no common ancestor and can only see public bindings.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Audience {
    Public,
    Within(String, Vec<String>),
}

impl Audience {
    fn include(&mut self, module: &Module) {
        if let Self::Within(krate, path) = self {
            if *krate == module.krate {
                path.truncate(
                    path.iter()
                        .zip(&module.path)
                        .take_while(|(a, b)| a == b)
                        .count(),
                );
            } else {
                *self = Self::Public;
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct LookupKey {
    module: usize,
    name: String,
    namespace: Namespace,
    audience: Audience,
}

#[derive(Default)]
struct Lookups {
    active: HashSet<LookupKey>,
    memo: HashMap<LookupKey, Binding>,
    cycle_cuts: usize,
    path_depth: usize,
}

struct GlobLookup {
    key: LookupKey,
    globs: std::vec::IntoIter<(Vec<String>, bool)>,
    cycle_cuts: usize,
    candidate: Binding,
    indexed: Option<Found>,
}

type ModuleLoader<'a> = dyn FnMut(&str, &[String]) -> Result<Vec<Item>, String> + 'a;
type ItemFilter<'a> = dyn Fn(&Item) -> bool + 'a;

/// The indexed crates' modules, parsed as they are first reached.
pub struct Index<'a> {
    root: &'a Path,
    /// The crates it indexes: crate name and source directory below `root`.
    crates: &'static [(&'static str, &'static str)],
    /// Sources included into an inline module with `include!`, by file name.
    included: &'a [(&'a str, &'a str)],
    pub modules: Vec<Module>,
    by_path: HashMap<(String, Vec<String>), usize>,
    /// Every source file read, for the build script to track.
    pub read: std::cell::RefCell<Vec<std::path::PathBuf>>,
    /// Rewrites a source file's text before it is parsed: the game's generated module lists.
    expand: fn(&Path, &str) -> Result<String, String>,
    /// Crates added for one consumer beside its crates ([`Index::add_crate`]).
    extra: Vec<(String, String)>,
    /// A crate's root file where it is not `lib.rs` (a library's `[lib] path`).
    root_files: HashMap<String, String>,
    /// Cargo names for dependencies whose source is in this index, by importing crate.
    dependency_aliases: HashMap<String, HashMap<String, String>>,
    default_features: HashMap<String, HashSet<String>>,
    load_module: Option<Box<ModuleLoader<'a>>>,
    item_enabled: Option<Box<ItemFilter<'a>>>,
}

/// The file a module's `#[path = "..."]` names.
fn path_attribute(attrs: &[syn::Attribute], features: Option<&HashSet<String>>) -> Option<String> {
    fn path(meta: &Meta, features: Option<&HashSet<String>>) -> Option<String> {
        match meta {
            Meta::NameValue(value) if value.path.is_ident("path") => match &value.value {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(file),
                    ..
                }) => Some(file.value()),
                _ => None,
            },
            Meta::List(list) if list.path.is_ident("cfg_attr") => {
                let args = list
                    .parse_args_with(
                        syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
                    )
                    .ok()?;
                let condition = args.first()?;
                (cfg_active(condition, features) != Some(false))
                    .then(|| args.iter().skip(1).find_map(|meta| path(meta, features)))
                    .flatten()
            }
            _ => None,
        }
    }

    attrs.iter().find_map(|attr| path(&attr.meta, features))
}

/// The directory below the crate root that holds `path`'s last module, with its slash.
fn parent_dir(path: &[String]) -> String {
    path.split_last().map_or(String::new(), |(_, dir)| {
        dir.iter().map(|d| format!("{d}/")).collect()
    })
}

impl<'a> Index<'a> {
    pub fn new(
        root: &'a Path,
        included: &'a [(&'a str, &'a str)],
        crates: &'static [(&'static str, &'static str)],
    ) -> Self {
        Self {
            root,
            crates,
            expand: |_, source| Ok(source.to_owned()),
            included,
            modules: Vec::new(),
            by_path: HashMap::new(),
            read: Default::default(),
            extra: Vec::new(),
            root_files: HashMap::new(),
            dependency_aliases: HashMap::new(),
            default_features: HashMap::new(),
            load_module: None,
            item_enabled: None,
        }
    }

    /// Names the file `krate`'s root module lives in, below its source directory.
    pub fn set_root_file(&mut self, krate: &str, file: &str) {
        self.root_files.insert(krate.to_owned(), file.to_owned());
    }

    /// Reads each source file through `expand` before parsing it.
    pub fn expanding(mut self, expand: fn(&Path, &str) -> Result<String, String>) -> Self {
        self.expand = expand;
        self
    }

    /// Indexes one more crate, for this index only: the surface walk reads the facade crate
    /// (`sunrise_mod_api`), which no generated output resolves through.
    pub fn add_crate(&mut self, name: &str, dir: &str) {
        if self.crate_dir(name).is_none() {
            self.extra.push((name.to_owned(), dir.to_owned()));
        }
    }

    pub fn add_dependency_aliases(&mut self, aliases: &HashMap<String, HashMap<String, String>>) {
        self.dependency_aliases.extend(aliases.clone());
    }

    pub fn add_default_features(&mut self, features: &HashMap<String, HashSet<String>>) {
        self.default_features.extend(features.clone());
    }

    /// Resolve function items using the caller's view of the active module tree.
    pub fn loading_functions(
        mut self,
        load: impl FnMut(&str, &[String]) -> Result<Vec<Item>, String> + 'a,
    ) -> Self {
        self.load_module = Some(Box::new(load));
        self
    }

    /// Filter items lazily when name resolution actually matches them, rather than when a module
    /// is loaded.
    pub fn filtering_items(mut self, enabled: impl Fn(&Item) -> bool + 'a) -> Self {
        self.item_enabled = Some(Box::new(enabled));
        self
    }

    fn item_active(&self, module: usize, item: &Item, attrs: &[Attribute]) -> bool {
        match &self.item_enabled {
            Some(enabled) => enabled(item),
            None => self.default_build(module, attrs),
        }
    }

    /// Whether an item's attributes admit the default build of its owning crate.
    pub fn default_build(&self, module: usize, attrs: &[Attribute]) -> bool {
        let features = self.default_features.get(&self.modules[module].krate);
        attrs
            .iter()
            .all(|attr| attribute_is_active(&attr.meta, features))
    }

    fn extern_crate_name(&mut self, module: usize, name: &str) -> Result<Option<String>, String> {
        let krate = self.modules[module].krate.clone();
        let root = self.module(&krate, &[])?;
        let spelling = self.modules[root].items.iter().find_map(|item| {
            let Item::ExternCrate(extern_item) = item else {
                return None;
            };
            let alias = extern_item
                .rename
                .as_ref()
                .map_or(&extern_item.ident, |(_, name)| name);
            (*alias == name && self.item_active(root, item, &extern_item.attrs))
                .then(|| extern_item.ident.to_string())
        });
        if spelling.as_deref() == Some("self") {
            return Ok(Some(krate));
        }
        let name = spelling.as_deref().unwrap_or(name);
        Ok(self
            .dependency_aliases
            .get(&krate)
            .and_then(|aliases| aliases.get(name))
            .cloned()
            .or_else(|| self.crate_name(name)))
    }

    fn crate_name(&self, name: &str) -> Option<String> {
        self.crates
            .iter()
            .find(|(krate, _)| *krate == name)
            .map(|(krate, _)| (*krate).to_owned())
            .or_else(|| {
                self.extra
                    .iter()
                    .find(|(krate, _)| krate == name)
                    .map(|(krate, _)| krate.clone())
            })
    }

    fn crate_dir(&self, name: &str) -> Option<(&str, &str)> {
        self.crates
            .iter()
            .find(|(krate, _)| *krate == name)
            .map(|(krate, dir)| (*krate, *dir))
            .or_else(|| {
                self.extra
                    .iter()
                    .find(|(krate, _)| krate == name)
                    .map(|(krate, dir)| (krate.as_str(), dir.as_str()))
            })
    }

    pub fn item(&self, id: ItemId) -> &Item {
        &self.modules[id.module].items[id.item]
    }

    /// The owning crate, defining module and original name of an indexed item.
    pub fn item_path(&self, id: ItemId) -> Option<Vec<String>> {
        let module = &self.modules[id.module];
        let name = match self.item(id) {
            Item::Const(item) => &item.ident,
            Item::Enum(item) => &item.ident,
            Item::Fn(item) => &item.sig.ident,
            Item::Static(item) => &item.ident,
            Item::Struct(item) => &item.ident,
            Item::Trait(item) => &item.ident,
            Item::Type(item) => &item.ident,
            Item::Union(item) => &item.ident,
            _ => return None,
        };
        Some(
            std::iter::once(module.krate.clone())
                .chain(module.path.iter().cloned())
                .chain(std::iter::once(name.to_string()))
                .collect(),
        )
    }

    /// The owning crate and the path below its root for a resolved module.
    pub fn module_path(&self, module: usize) -> Vec<String> {
        let Module { krate, path, .. } = &self.modules[module];
        std::iter::once(krate.clone())
            .chain(path.iter().cloned())
            .collect()
    }

    /// The module at `path` in `krate`, parsed on first use.
    pub fn module(&mut self, krate: impl AsRef<str>, path: &[String]) -> Result<usize, String> {
        let requested = krate.as_ref();
        let krate = self
            .crate_name(requested)
            .ok_or_else(|| format!("{requested} is not a crate the mod API builds on"))?;
        if let Some(&id) = self.by_path.get(&(krate.clone(), path.to_vec())) {
            return Ok(id);
        }
        let items = if let Some(load) = &mut self.load_module {
            load(&krate, path)?
        } else {
            self.parse_module(&krate, path)?
        };
        let id = self.modules.len();
        self.modules.push(Module {
            krate: krate.clone(),
            path: path.to_vec(),
            items,
        });
        self.by_path.insert((krate, path.to_vec()), id);
        Ok(id)
    }

    fn parse_module(&mut self, krate: &str, path: &[String]) -> Result<Vec<Item>, String> {
        Ok(match path.split_last() {
            None => {
                let file = self.root_files.get(krate).map_or("lib.rs", String::as_str);
                self.parse(krate, file)?
            }
            Some((last, parent)) => {
                let parent = self.module(krate, parent)?;
                let declarations = &self.modules[parent].items;
                let has_declaration = declarations
                    .iter()
                    .any(|item| matches!(item, Item::Mod(item) if item.ident == last.as_str()));
                let declared = declarations.iter().find_map(|item| match item {
                    Item::Mod(item)
                        if item.ident == last.as_str()
                            && self.default_build(parent, &item.attrs) =>
                    {
                        Some((
                            item.content.clone(),
                            path_attribute(&item.attrs, self.default_features.get(krate)),
                        ))
                    }
                    _ => None,
                });
                if declared.is_none() && has_declaration {
                    return Err(format!(
                        "{}::{} is excluded from the default build",
                        krate,
                        path.join("::")
                    ));
                }
                match declared {
                    Some((Some((_, items)), _)) => self.expand_includes(items)?,
                    // A generated `#[path]` is absolute.
                    Some((None, Some(file))) if Path::new(&file).is_absolute() => {
                        self.parse(krate, &file)?
                    }
                    // `#[path]` is relative to the declaring file's directory: `parent.rs`
                    // sits beside it, `parent/mod.rs` below.
                    Some((None, Some(file))) => {
                        let beside = parent_dir(&path[..path.len() - 1]);
                        let below = parent_dir(path);
                        self.parse(krate, &format!("{beside}{file}"))
                            .or_else(|_| self.parse(krate, &format!("{below}{file}")))?
                    }
                    _ => {
                        let dir = path.join("/");
                        self.parse(krate, &format!("{dir}.rs"))
                            .or_else(|_| self.parse(krate, &format!("{dir}/mod.rs")))?
                    }
                }
            }
        })
    }

    fn parse(&self, krate: &str, file: &str) -> Result<Vec<Item>, String> {
        let dir = self.crate_dir(krate).expect("a known crate").1;
        let path = self.root.join(dir).join(file);
        let source = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.read.borrow_mut().push(path.clone());
        let source = (self.expand)(self.root, &source)?;
        let file = syn::parse_file(&source).map_err(|e| format!("{}: {e}", path.display()))?;
        self.expand_includes(file.items)
    }

    /// `items` with each `include!` of a generated file replaced by that file's items.
    fn expand_includes(&self, items: Vec<Item>) -> Result<Vec<Item>, String> {
        let mut out = Vec::new();
        for item in items {
            match &item {
                Item::Macro(mac) if mac.mac.path.is_ident("include") => {
                    let tokens = mac.mac.tokens.to_string();
                    // The wire impls are generated from what this reads, so it never reads them.
                    if tokens.contains("wire_") {
                        continue;
                    }
                    let (name, source) = self
                        .included
                        .iter()
                        .find(|(name, _)| tokens.contains(name))
                        .ok_or_else(|| format!("include!({tokens}) is not a generated file"))?;
                    let file = syn::parse_file(source).map_err(|e| format!("{name}: {e}"))?;
                    out.extend(file.items);
                }
                _ => out.push(item),
            }
        }
        Ok(out)
    }

    /// What `name` means in `module`: an item defined there, a `use` of it, or a glob import
    /// that provides it.
    pub fn resolve(&mut self, module: usize, name: &str) -> Result<Found, String> {
        self.lookup_here(module, name, Namespace::Any, &mut Lookups::default())?
            .require(self, module, name)
    }

    /// What type `name` means in `module`, following named and glob imports in the type namespace.
    /// A named import of a value does not hide a type provided by a glob import.
    pub fn resolve_type(&mut self, module: usize, name: &str) -> Result<Found, String> {
        self.lookup_here(module, name, Namespace::Type, &mut Lookups::default())?
            .require(self, module, name)
    }

    fn lookup_here(
        &mut self,
        module: usize,
        name: &str,
        namespace: Namespace,
        lookups: &mut Lookups,
    ) -> Result<Binding, String> {
        let m = &self.modules[module];
        self.lookup(
            LookupKey {
                module,
                name: name.to_owned(),
                namespace,
                audience: Audience::Within(m.krate.clone(), m.path.clone()),
            },
            lookups,
        )
    }

    fn lookup(&mut self, key: LookupKey, lookups: &mut Lookups) -> Result<Binding, String> {
        let mut stack: Vec<GlobLookup> = Vec::new();
        let mut next = Some(key);
        let mut result = Binding::Missing;
        loop {
            if let Some(key) = next.take() {
                if let Some(found) = lookups.memo.get(&key) {
                    result = found.clone();
                } else if !lookups.active.insert(key.clone()) {
                    lookups.cycle_cuts += 1;
                    result = Binding::Missing;
                } else {
                    let cycle_cuts = lookups.cycle_cuts;
                    let mut globs = Vec::new();
                    let candidate = self.explicit_binding(&key, lookups, &mut globs)?;
                    result = Binding::Missing;
                    stack.push(GlobLookup {
                        key,
                        globs: globs.into_iter(),
                        cycle_cuts,
                        candidate,
                        indexed: None,
                    });
                }
            }
            let Some(frame) = stack.last_mut() else {
                return Ok(result);
            };
            match std::mem::replace(&mut result, Binding::Missing) {
                found @ Binding::Found(_) if frame.key.namespace == Namespace::Any => {
                    frame.candidate = found;
                }
                Binding::Found(found) => {
                    if matches!(found, Found::Item(_) | Found::Module(_)) {
                        if let Some(first) = &frame.indexed {
                            if *first != found {
                                let path = |found: &Found| match found {
                                    Found::Item(id) => self.item_path(*id).unwrap_or_default(),
                                    Found::Module(module) => self.module_path(*module),
                                    Found::External { path, .. } => path.clone(),
                                };
                                frame.candidate = Binding::Ambiguous(ambiguous_glob(
                                    &frame.key.name,
                                    &path(first),
                                    &path(&found),
                                ));
                            }
                        } else {
                            frame.indexed = Some(found.clone());
                        }
                    }
                    if matches!(frame.candidate, Binding::Missing) {
                        frame.candidate = Binding::Found(found);
                    }
                }
                ambiguous @ Binding::Ambiguous(..) => frame.candidate = ambiguous,
                // A hidden glob binding shadows names in its own module only.
                Binding::Missing | Binding::Hidden => {}
            }
            let finished = match frame.candidate {
                Binding::Found(_) => frame.key.namespace == Namespace::Any,
                Binding::Ambiguous(..) | Binding::Hidden => true,
                Binding::Missing => false,
            };
            if !finished {
                for (glob, rooted) in frame.globs.by_ref() {
                    let binding = self.resolve_path_in_namespace(
                        frame.key.module,
                        &glob,
                        rooted,
                        Namespace::Type,
                        lookups,
                    )?;
                    if matches!(binding, Binding::Ambiguous(..)) {
                        frame.candidate = binding;
                        break;
                    }
                    if let Binding::Found(Found::Module(target)) = binding {
                        let mut key = frame.key.clone();
                        key.audience.include(&self.modules[key.module]);
                        key.module = target;
                        next = Some(key);
                        break;
                    }
                }
                if next.is_some() {
                    continue;
                }
            }
            let Some(frame) = stack.pop() else {
                return Ok(result);
            };
            result = frame.candidate;
            lookups.active.remove(&frame.key);
            // A branch cut by an active ancestor is not a complete lookup, even if another
            // branch supplied a binding. It may resolve differently along another route.
            if lookups.cycle_cuts == frame.cycle_cuts {
                lookups.memo.insert(frame.key, result.clone());
            }
        }
    }

    fn explicit_binding(
        &mut self,
        key: &LookupKey,
        lookups: &mut Lookups,
        globs: &mut Vec<(Vec<String>, bool)>,
    ) -> Result<Binding, String> {
        let LookupKey {
            module,
            name,
            namespace,
            audience,
        } = key;
        let (module, name, namespace) = (*module, name.as_str(), *namespace);
        let items = &self.modules[module].items;
        let reachable = |vis| self.visible_to(module, vis, audience);
        let mut shadowed = false;
        for (item, definition_item) in items.iter().enumerate() {
            if let Some((attrs, visibility)) = definition(definition_item)
                && defines(definition_item, name, namespace)
                && self.item_active(module, definition_item, attrs)
            {
                if reachable(visibility) {
                    if let Item::Mod(child) = definition_item {
                        let krate = self.modules[module].krate.clone();
                        let mut path = self.modules[module].path.clone();
                        path.push(child.ident.to_string());
                        return self
                            .module(&krate, &path)
                            .map(|target| Binding::Found(Found::Module(target)));
                    }
                    return Ok(Binding::Found(Found::Item(ItemId { module, item })));
                }
                shadowed = true;
            }
        }
        let mut named = Vec::new();
        for item in items {
            match item {
                Item::Use(use_item) => {
                    let open = reachable(&use_item.vis);
                    let leading_colon = use_item.leading_colon.is_some();
                    let mut matched_named = Vec::new();
                    let mut matched_globs = Vec::new();
                    uses(
                        &use_item.tree,
                        &mut Vec::new(),
                        &mut |path, alias| match alias {
                            Some(alias) if alias == name => {
                                matched_named.push((path, leading_colon, open));
                            }
                            None if open => matched_globs.push((path, leading_colon)),
                            _ => {}
                        },
                    );
                    if (!matched_named.is_empty() || !matched_globs.is_empty())
                        && self.item_active(module, item, &use_item.attrs)
                    {
                        named.extend(matched_named);
                        globs.extend(matched_globs);
                    }
                }
                Item::ExternCrate(extern_item) => {
                    let alias = extern_item
                        .rename
                        .as_ref()
                        .map_or(&extern_item.ident, |(_, alias)| alias);
                    if *alias == name && self.item_active(module, item, &extern_item.attrs) {
                        let (source, rooted) = if extern_item.ident == "self" {
                            ("crate".to_owned(), false)
                        } else {
                            (extern_item.ident.to_string(), true)
                        };
                        named.push((vec![source], rooted, reachable(&extern_item.vis)));
                    }
                }
                _ => {}
            }
        }
        if namespace == Namespace::Any
            && let Some((path, rooted, open)) = named.first()
        {
            globs.clear();
            return if *open {
                self.resolve_path_in_namespace(module, path, *rooted, namespace, lookups)
            } else {
                Ok(Binding::Hidden)
            };
        }
        let mut external_named = None;
        for (path, rooted, open) in named {
            match self.resolve_path_in_namespace(module, &path, rooted, Namespace::Type, lookups)? {
                found @ (Binding::Found(Found::Item(_) | Found::Module(_))
                | Binding::Ambiguous(..))
                    if open =>
                {
                    globs.clear();
                    return Ok(found);
                }
                Binding::Found(found @ Found::External { .. }) if open => {
                    // An unindexed import's namespace is unknown. Preserve the external path
                    // so the caller can handle that without mistaking it for a local glob type.
                    external_named.get_or_insert(found);
                }
                Binding::Found(_) | Binding::Ambiguous(..) => shadowed = true,
                Binding::Missing | Binding::Hidden => {}
            }
        }
        if shadowed || external_named.is_some() {
            globs.clear();
        }
        Ok(if shadowed {
            Binding::Hidden
        } else {
            external_named.map_or(Binding::Missing, Binding::Found)
        })
    }

    /// A restricted item is visible only in the named module and its descendants.
    fn visible_to(&self, module: usize, visibility: &Visibility, audience: &Audience) -> bool {
        let scope = &self.modules[module];
        match (visibility, audience) {
            (Visibility::Public(_), _) => true,
            (_, Audience::Public) => false,
            (Visibility::Inherited, Audience::Within(krate, path)) => {
                scope.krate == *krate && path.starts_with(&scope.path)
            }
            (Visibility::Restricted(visibility), Audience::Within(krate, path)) => {
                let written = visibility
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect::<Vec<_>>();
                self.local_absolute(module, &written)
                    .is_some_and(|(owner, prefix)| owner == *krate && path.starts_with(&prefix))
            }
        }
    }

    /// What `path`, written inside a block with local `use` items `imports`, names.
    pub fn resolve_with_imports(
        &mut self,
        module: usize,
        path: &[String],
        imports: &[&UseTree],
    ) -> Result<Found, String> {
        if imports.is_empty() {
            return self.resolve_path(module, path);
        }
        let mut path = path.to_vec();
        let mut seen = std::collections::HashSet::new();
        while let Some(first) = path.first() {
            let mut named = None;
            for tree in imports {
                uses(tree, &mut Vec::new(), &mut |prefix, alias| {
                    if named.is_none() && alias.as_ref() == Some(first) {
                        named = Some(prefix);
                    }
                });
                if named.is_some() {
                    break;
                }
            }
            let Some(mut prefix) = named else { break };
            if !seen.insert(first.clone()) {
                return Err(format!("{first} imports itself"));
            }
            let same_name = prefix.first() == Some(first);
            prefix.extend_from_slice(&path[1..]);
            path = prefix;
            if same_name {
                break;
            }
        }
        self.resolve_path(module, &path)
    }

    /// What the path `path`, written in `module`, names.
    pub fn resolve_path(&mut self, module: usize, path: &[String]) -> Result<Found, String> {
        self.resolve_path_in_namespace(
            module,
            path,
            false,
            Namespace::Any,
            &mut Lookups::default(),
        )?
        .require(self, module, path.last().map_or("", String::as_str))
    }

    /// What the signature type path `path`, written in `module`, names.
    pub fn resolve_type_path(&mut self, module: usize, path: &[String]) -> Result<Found, String> {
        self.resolve_path_in_namespace(
            module,
            path,
            false,
            Namespace::Type,
            &mut Lookups::default(),
        )?
        .require(self, module, path.last().map_or("", String::as_str))
    }

    /// What a `::`-rooted import names, skipping local bindings of the crate name.
    pub fn resolve_rooted_path(&mut self, module: usize, path: &[String]) -> Result<Found, String> {
        self.resolve_path_in_namespace(module, path, true, Namespace::Any, &mut Lookups::default())?
            .require(self, module, path.last().map_or("", String::as_str))
    }

    fn resolve_path_in_namespace(
        &mut self,
        module: usize,
        path: &[String],
        rooted: bool,
        namespace: Namespace,
        lookups: &mut Lookups,
    ) -> Result<Binding, String> {
        // Glob edges use the explicit DFS stack. Bound the remaining recursive path/alias
        // expansion against pathological source rather than treating it as an absent binding.
        if lookups.path_depth >= 128 {
            let scope = &self.modules[module];
            return Err(format!(
                "import path nesting limit resolving `{}` in {}::{}",
                path.join("::"),
                scope.krate,
                scope.path.join("::")
            ));
        }
        lookups.path_depth += 1;
        let result = self.follow_path(module, path, rooted, namespace, lookups);
        lookups.path_depth -= 1;
        result
    }

    fn follow_path(
        &mut self,
        module: usize,
        path: &[String],
        rooted: bool,
        namespace: Namespace,
        lookups: &mut Lookups,
    ) -> Result<Binding, String> {
        let (first, suffix) = path.split_first().ok_or("an empty item path")?;
        let mut scope = module;
        let mut remaining = path;
        let mut unqualified = false;
        if rooted {
            let Some(krate) = self.extern_crate_name(module, first)? else {
                return Ok(Binding::Found(Found::External {
                    module,
                    path: path.to_vec(),
                    rooted,
                }));
            };
            scope = self.module(krate, &[])?;
            remaining = suffix;
        } else {
            match first.as_str() {
                "crate" => {
                    scope = self.module(self.modules[module].krate.clone(), &[])?;
                    remaining = suffix;
                }
                "self" => remaining = suffix,
                "super" => {
                    let count = path
                        .iter()
                        .take_while(|segment| *segment == "super")
                        .count();
                    let m = &self.modules[module];
                    let Some(length) = m.path.len().checked_sub(count) else {
                        return Ok(Binding::Missing);
                    };
                    let prefix = Vec::from(&m.path[..length]);
                    scope = self.module(m.krate.clone(), &prefix)?;
                    remaining = &path[count..];
                }
                _ => unqualified = true,
            }
        }
        let origin = &self.modules[module];
        let audience = Audience::Within(origin.krate.clone(), origin.path.clone());
        while let Some((segment, rest)) = remaining.split_first() {
            let binding = self.lookup(
                LookupKey {
                    module: scope,
                    name: segment.clone(),
                    namespace: if rest.is_empty() {
                        namespace
                    } else {
                        Namespace::Type
                    },
                    audience: audience.clone(),
                },
                lookups,
            )?;
            let binding = if unqualified && matches!(binding, Binding::Missing) {
                if let Some(krate) = self.extern_crate_name(module, segment)? {
                    Binding::Found(Found::Module(self.module(krate, &[])?))
                } else if !rest.is_empty() {
                    return Ok(Binding::Found(Found::External {
                        module,
                        path: path.to_vec(),
                        rooted,
                    }));
                } else {
                    Binding::Missing
                }
            } else {
                binding
            };
            match binding {
                Binding::Found(Found::Module(target)) => scope = target,
                Binding::Found(Found::External {
                    module,
                    path: mut prefix,
                    rooted,
                }) => {
                    prefix.extend_from_slice(rest);
                    return Ok(Binding::Found(Found::External {
                        module,
                        path: prefix,
                        rooted,
                    }));
                }
                Binding::Found(Found::Item(_)) if !rest.is_empty() => return Ok(Binding::Missing),
                other => return Ok(other),
            }
            unqualified = false;
            remaining = rest;
        }
        Ok(Binding::Found(Found::Module(scope)))
    }

    /// The type-namespace binding of a complete path, retaining shadowing and ambiguity.
    pub fn resolve_type_binding(
        &mut self,
        module: usize,
        path: &[String],
        rooted: bool,
    ) -> Result<Binding, String> {
        self.resolve_path_in_namespace(
            module,
            path,
            rooted,
            Namespace::Type,
            &mut Lookups::default(),
        )
    }

    /// Resolves `path` as a module using the same namespace as signature types.
    pub fn resolve_module_path(
        &mut self,
        module: usize,
        path: &[String],
    ) -> Result<Option<usize>, String> {
        match self.resolve_type_binding(module, path, false)? {
            Binding::Found(Found::Module(module)) => Ok(Some(module)),
            Binding::Ambiguous(error) => Err(error),
            _ => Ok(None),
        }
    }

    /// A path to a declared local module, or one starting at `crate`, `self` or `super`.
    fn local_absolute(&self, module: usize, path: &[String]) -> Option<(String, Vec<String>)> {
        let m = &self.modules[module];
        let (first, rest) = path.split_first()?;
        let mut base = match first.as_str() {
            "crate" => (m.krate.clone(), Vec::new()),
            "self" => (m.krate.clone(), m.path.clone()),
            "super" => (
                m.krate.clone(),
                m.path[..m.path.len().checked_sub(1)?].to_vec(),
            ),
            name if m.items.iter().any(|item| {
                matches!(item, Item::Mod(mod_item) if mod_item.ident == name && self.item_active(module, item, &mod_item.attrs))
            }) =>
            {
                let mut path = m.path.clone();
                path.push(name.to_string());
                (m.krate.clone(), path)
            }
            _ => return None,
        };
        for segment in rest {
            if segment == "super" {
                base.1.pop()?;
            } else {
                base.1.push(segment.clone());
            }
        }
        Some(base)
    }
}

/// Describes two distinct definitions imported into a type namespace through globs.
fn ambiguous_glob(name: &str, first: &[String], second: &[String]) -> String {
    let first = first.join("::");
    let second = second.join("::");
    format!(
        "ambiguous type-namespace binding `{name}`: glob imports supply both `{first}` and `{second}`; qualify the intended path explicitly"
    )
}

/// Whether `item` defines `name` in the requested namespace.
fn defines(item: &Item, name: &str, namespace: Namespace) -> bool {
    if matches!(namespace, Namespace::Type) {
        return is_type_item_named(item, name);
    }
    match item {
        Item::Mod(item) => item.ident == name,
        Item::Struct(item) => item.ident == name,
        Item::Enum(item) => item.ident == name,
        Item::Type(item) => item.ident == name,
        Item::Trait(item) => item.ident == name,
        Item::Union(item) => item.ident == name,
        Item::Const(item) => item.ident == name,
        Item::Fn(item) => item.sig.ident == name,
        Item::Static(item) => item.ident == name,
        _ => false,
    }
}

fn is_type_item_named(item: &Item, name: &str) -> bool {
    match item {
        Item::Mod(item) => item.ident == name,
        Item::Struct(item) => item.ident == name,
        Item::Enum(item) => item.ident == name,
        Item::Type(item) => item.ident == name,
        Item::Trait(item) => item.ident == name,
        Item::Union(item) => item.ident == name,
        _ => false,
    }
}

fn definition(item: &Item) -> Option<(&[Attribute], &Visibility)> {
    let (attrs, visibility) = match item {
        Item::Const(item) => (&item.attrs, &item.vis),
        Item::Fn(item) => (&item.attrs, &item.vis),
        Item::Mod(item) => (&item.attrs, &item.vis),
        Item::Enum(item) => (&item.attrs, &item.vis),
        Item::Struct(item) => (&item.attrs, &item.vis),
        Item::Static(item) => (&item.attrs, &item.vis),
        Item::Trait(item) => (&item.attrs, &item.vis),
        Item::Type(item) => (&item.attrs, &item.vis),
        Item::Union(item) => (&item.attrs, &item.vis),
        _ => return None,
    };
    Some((attrs, visibility))
}

/// Each path `tree` imports, with the name it binds (`None` for a glob).
pub fn uses(
    tree: &UseTree,
    prefix: &mut Vec<String>,
    f: &mut impl FnMut(Vec<String>, Option<String>),
) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            uses(&path.tree, prefix, f);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut path = prefix.clone();
            if name.ident != "self" {
                path.push(name.ident.to_string());
            }
            let alias = path.last().cloned();
            f(path, alias);
        }
        UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            if rename.ident != "self" {
                path.push(rename.ident.to_string());
            }
            f(path, Some(rename.rename.to_string()));
        }
        UseTree::Glob(_) => f(prefix.clone(), None),
        UseTree::Group(group) => {
            for tree in &group.items {
                uses(tree, prefix, f);
            }
        }
    }
}

#[cfg(test)]
mod index_tests {
    use super::*;
    use std::fs;

    /// Resolve a name in a module of a one-crate test index.
    fn resolves(lib: &str, module: &[&str], name: &str) -> Option<Vec<String>> {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("k/src")).unwrap();
        fs::write(dir.path().join("k/src/lib.rs"), lib).unwrap();
        let mut index = Index::new(dir.path(), &[], &[("k", "k/src")]);
        let path: Vec<String> = module.iter().map(|m| (*m).to_owned()).collect();
        let module = index.module("k", &path).unwrap();
        match index.resolve(module, name) {
            Ok(Found::Item(id)) => Some(index.modules[id.module].path.clone()),
            Ok(Found::Module(_)) => panic!("{name} is a module"),
            Ok(Found::External { path, .. }) => panic!("{name} left the crate: {path:?}"),
            Err(_) => None,
        }
    }

    #[test]
    fn default_build_uses_debug_and_thread_local_cfgs() {
        let debug: Attribute = syn::parse_quote!(#[cfg(debug_assertions)]);
        assert_eq!(
            attribute_is_active(&debug.meta, None),
            cfg!(debug_assertions)
        );

        let thread_local: Attribute = syn::parse_quote!(#[cfg(target_thread_local)]);
        let family_supports_thread_local = target_cfg("target_family").is_some_and(|family| {
            family
                .split(',')
                .any(|name| name == "unix" || name == "windows")
        });
        assert_eq!(
            attribute_is_active(&thread_local.meta, None),
            target_flag("target_thread_local", family_supports_thread_local)
        );

        let test_only: Attribute = syn::parse_quote!(#[cfg(test)]);
        assert!(!attribute_is_active(&test_only.meta, None));
    }

    #[test]
    fn an_unknown_target_feature_keeps_the_item_under_any_negation() {
        for attr in [
            quote::quote!(#[cfg(target_feature = "sunrise-fixture")]),
            quote::quote!(#[cfg(not(target_feature = "sunrise-fixture"))]),
            quote::quote!(#[cfg(not(all(unix, target_feature = "sunrise-fixture")))]),
            quote::quote!(#[cfg_attr(target_feature = "sunrise-fixture", cfg(any()))]),
            quote::quote!(#[cfg_attr(not(target_feature = "sunrise-fixture"), cfg(any()))]),
        ] {
            let text = attr.to_string();
            let attr: Attribute = syn::parse_quote!(#attr);
            assert!(attribute_is_active(&attr.meta, None), "{text}");
        }
        // A known operand still decides.
        for (attr, active) in [
            (
                quote::quote!(#[cfg(all(test, target_feature = "sunrise-fixture"))]),
                false,
            ),
            (
                quote::quote!(#[cfg(not(any(not(test), target_feature = "sunrise-fixture")))]),
                false,
            ),
        ] {
            let text = attr.to_string();
            let attr: Attribute = syn::parse_quote!(#attr);
            assert_eq!(attribute_is_active(&attr.meta, None), active, "{text}");
        }
    }

    #[test]
    fn module_uses_the_active_cfg_attr_path() {
        const CRATES: &[(&str, &str)] = &[("engine", "engine/src")];
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("engine/src/platform")).unwrap();
        fs::write(
            root.join("engine/src/lib.rs"),
            "#[cfg_attr(unix, cfg_attr(all(), path = \"platform/unix.rs\"))]\n#[cfg_attr(windows, path = \"platform/windows.rs\")]\n#[cfg_attr(not(any(unix, windows)), path = \"platform/other.rs\")]\npub mod platform;\n",
        )
        .unwrap();
        fs::write(
            root.join("engine/src/platform/unix.rs"),
            "pub struct UnixPlatform;\n",
        )
        .unwrap();
        fs::write(
            root.join("engine/src/platform/windows.rs"),
            "pub struct WindowsPlatform;\n",
        )
        .unwrap();
        fs::write(
            root.join("engine/src/platform/other.rs"),
            "pub struct OtherPlatform;\n",
        )
        .unwrap();

        let mut index = Index::new(root, &[], CRATES);
        let module = index.module("engine", &["platform".to_owned()]).unwrap();
        let expected = if cfg!(windows) {
            "WindowsPlatform"
        } else if cfg!(unix) {
            "UnixPlatform"
        } else {
            "OtherPlatform"
        };
        assert!(matches!(
            index.resolve(module, expected),
            Ok(Found::Item(_))
        ));
    }

    #[test]
    fn type_resolution_skips_named_const_and_static_imports_before_globs() {
        const CRATES: &[(&str, &str)] = &[("consumer", "consumer/src"), ("engine", "engine/src")];
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("consumer/src")).unwrap();
        fs::create_dir_all(root.join("engine/src")).unwrap();
        fs::write(
            root.join("consumer/src/lib.rs"),
            "const LOCAL_CONST: u8 = 0;\nstatic LOCAL_STATIC: u8 = 0;\nuse crate::LOCAL_CONST as ConstName;\nuse crate::LOCAL_STATIC as StaticName;\nuse engine::*;\n",
        )
        .unwrap();
        fs::write(
            root.join("engine/src/lib.rs"),
            "pub struct ConstName;\npub struct StaticName;\n",
        )
        .unwrap();

        let mut index = Index::new(root, &[], CRATES);
        let consumer = index.module("consumer", &[]).unwrap();
        for name in ["ConstName", "StaticName"] {
            let Found::Item(id) = index.resolve_type(consumer, name).unwrap() else {
                panic!("{name} should resolve to the globbed type");
            };
            assert_eq!(index.modules[id.module].krate, "engine");
            assert!(matches!(index.item(id), Item::Struct(item) if item.ident == name));
        }

        let Found::Item(id) = index.resolve(consumer, "LOCAL_STATIC").unwrap() else {
            panic!("the static should remain resolvable as a value name");
        };
        assert!(matches!(index.item(id), Item::Static(item) if item.ident == "LOCAL_STATIC"));
    }

    #[test]
    fn type_resolution_prefers_a_named_type_import_over_a_glob() {
        const CRATES: &[(&str, &str)] = &[("consumer", "consumer/src"), ("engine", "engine/src")];
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("consumer/src")).unwrap();
        fs::create_dir_all(root.join("engine/src")).unwrap();
        fs::write(
            root.join("consumer/src/lib.rs"),
            "pub struct Explicit;\nuse crate::Explicit as Secret;\nuse engine::*;\n",
        )
        .unwrap();
        fs::write(root.join("engine/src/lib.rs"), "pub struct Secret;\n").unwrap();

        let mut index = Index::new(root, &[], CRATES);
        let consumer = index.module("consumer", &[]).unwrap();
        let Found::Item(id) = index
            .resolve_type_path(consumer, &["Secret".to_owned()])
            .unwrap()
        else {
            panic!("the explicit named type import should resolve to an indexed item");
        };
        assert_eq!(index.modules[id.module].krate, "consumer");
        assert!(matches!(index.item(id), Item::Struct(item) if item.ident == "Explicit"));
    }

    #[test]
    fn mixed_named_and_glob_cycles_leave_later_globs_available() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("k/src")).unwrap();
        fs::write(
            dir.path().join("k/src/lib.rs"),
            "mod left { pub use crate::middle::{Name, Shown}; }
             mod right { pub use crate::middle::{Name, Shown}; }
             mod middle { pub use crate::left::*; pub use crate::right::*; pub use crate::later::*; }
             mod later { pub struct Shown; }",
        )
        .unwrap();
        let mut index = Index::new(dir.path(), &[], &[("k", "k/src")]);
        let root = index.module("k", &[]).unwrap();
        assert!(
            index
                .resolve_type_path(root, &["middle".into(), "Name".into()])
                .is_err()
        );
        let Found::Item(id) = index
            .resolve_type_path(root, &["middle".into(), "Shown".into()])
            .unwrap()
        else {
            panic!("the later glob should supply Shown");
        };
        assert_eq!(index.modules[id.module].path, ["later"]);
    }

    #[test]
    fn a_glob_of_a_glob_imports_only_what_every_glob_holder_can_see() {
        let lib = "
            pub mod z {
                pub mod y { pub(super) struct Hidden; pub struct Shown; }
                pub mod w { use crate::*; }
            }
            pub use z::y::*;
        ";
        assert_eq!(
            resolves(lib, &["z", "w"], "Shown"),
            Some(vec!["z".into(), "y".into()])
        );
        assert_eq!(resolves(lib, &["z", "w"], "Hidden"), None);
    }

    #[test]
    fn a_definition_hidden_from_the_importer_still_shadows_its_modules_globs() {
        let lib = "
            pub mod a { struct Thing; pub use crate::b::*; }
            pub mod b { pub struct Thing; pub struct Other; }
            pub mod c { use crate::a::*; }
        ";
        assert_eq!(resolves(lib, &["c"], "Other"), Some(vec!["b".into()]));
        assert_eq!(resolves(lib, &["c"], "Thing"), None);
    }
}
