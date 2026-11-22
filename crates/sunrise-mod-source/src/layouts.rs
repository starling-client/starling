//! How a crate's wire types are described for the one shape function: each type's layout, from
//! how it crosses ([`WireForm`]) and its Rust fields, keyed by the path it is defined at (which
//! never enters a shape).
use crate::index::{Found, Index, ItemId};
use mod_surface::shape::{Body, Definition, Scalar, Shape};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use syn::visit_mut::VisitMut;
use syn::{Item, Type};

/// How one type crosses.
#[derive(Clone)]
pub enum WireForm {
    /// `#[derive(Wire)]`: its Rust fields.
    Derive,
    /// As another type, written in `module`.
    Via(Type, usize),
    /// A hand-written impl's declared fields, each type written in `module`.
    Fields(Vec<(String, Type)>, usize),
    /// A hand-written impl that declares no layout of its own.
    Manual,
}

impl WireForm {
    /// Its name in `wire_form`, `via:` followed by the normalised type.
    pub fn name(&self, via: impl FnOnce(&Type, usize) -> String) -> String {
        match self {
            Self::Derive => "derive".into(),
            Self::Via(ty, module) => format!("via:{}", via(ty, *module)),
            Self::Fields(..) => "fields".into(),
            Self::Manual => "manual".into(),
        }
    }
}

/// The traits of `attrs`' `#[derive(…)]` lists, by last path segment, sorted.
pub fn derives(attrs: &[syn::Attribute]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("derive")) {
        if let Ok(paths) = attr.parse_args_with(
            syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
        ) {
            for path in paths {
                if let Some(last) = path.segments.last() {
                    out.insert(last.ident.to_string());
                }
            }
        }
    }
    out.into_iter().collect()
}

/// A type or const parameter in declaration order.
#[derive(Clone)]
pub struct GenericName {
    pub name: String,
    pub is_type: bool,
}

/// The generic parameter names of `generics`, lifetimes left out.
pub fn param_names(generics: &syn::Generics) -> Vec<GenericName> {
    generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(t) => Some(GenericName {
                name: t.ident.to_string(),
                is_type: true,
            }),
            syn::GenericParam::Const(c) => Some(GenericName {
                name: c.ident.to_string(),
                is_type: false,
            }),
            syn::GenericParam::Lifetime(_) => None,
        })
        .collect()
}

/// Builds wire layout descriptions of the types the API's types contain, keyed by their
/// defining path, which never enters a shape. A type is described by how it crosses
/// ([`WireForm`]); one that does not cross, or crosses by a hand-written impl that declares no
/// layout, gets no definition, so every shape that contains it is refused.
pub struct Layouts<W> {
    pub table: BTreeMap<String, Definition>,
    keys: HashMap<ItemId, String>,
    pending: Vec<ItemId>,
    wires: W,
    /// A type that has no wire form of its own (a borrowed event, a view the boundary copies
    /// field by field) is described by its Rust fields, as a derive would.
    pub fields_without_form: bool,
}

/// What `Layouts` asks of the crate set it describes: how a type crosses, and how a type the
/// description does not look into is written.
pub trait Wires {
    /// How the type `id`, with these attributes and derives, crosses; none when it does not.
    fn form(&self, id: ItemId, attrs: &[syn::Attribute], derives: &[String]) -> Option<WireForm>;
    /// `id`'s public path, for a note.
    fn path(&self, id: ItemId) -> Option<&[String]>;
    /// `ty`, written in `module` with generic parameters `params`, as an opaque shape.
    fn opaque(
        &self,
        index: &mut Index<'_>,
        module: usize,
        ty: &Type,
        params: &[GenericName],
    ) -> Shape;
}

impl<W: Wires> Layouts<W> {
    pub fn new(wires: W) -> Self {
        Self {
            table: BTreeMap::new(),
            keys: HashMap::new(),
            pending: Vec::new(),
            wires,
            fields_without_form: false,
        }
    }

    /// The crate set's side of the description.
    pub fn wires(&self) -> &W {
        &self.wires
    }

    /// The key of `id`'s definition, scheduling it to be described.
    pub fn key(&mut self, index: &Index<'_>, id: ItemId) -> String {
        if let Some(key) = self.keys.get(&id) {
            return key.clone();
        }
        let module = &index.modules[id.module];
        let name = match index.item(id) {
            Item::Struct(i) => i.ident.to_string(),
            Item::Enum(i) => i.ident.to_string(),
            Item::Union(i) => i.ident.to_string(),
            _ => String::new(),
        };
        let key = std::iter::once(module.krate.to_owned())
            .chain(module.path.iter().cloned())
            .chain([name, format!("#{}", id.item)])
            .collect::<Vec<_>>()
            .join("::");
        self.keys.insert(id, key.clone());
        self.pending.push(id);
        key
    }

    /// The type a key names, for a note.
    pub fn named(&self, key: &str) -> String {
        let name = key.rsplit_once("::#").map_or(key, |(name, _)| name);
        self.keys
            .iter()
            .find(|(_, k)| k.as_str() == key)
            .and_then(|(id, _)| self.wires.path(*id))
            .map_or_else(|| name.to_owned(), |path| path.join("::"))
    }

    /// Describes every scheduled type and those they contain.
    pub fn finish(&mut self, index: &mut Index<'_>) {
        while let Some(id) = self.pending.pop() {
            let key = self.keys[&id].clone();
            let item = index.item(id).clone();
            let (attrs, generics) = match &item {
                Item::Struct(s) => (&s.attrs, &s.generics),
                Item::Enum(e) => (&e.attrs, &e.generics),
                Item::Union(u) => (&u.attrs, &u.generics),
                _ => continue,
            };
            let names = param_names(generics);
            let derives = derives(attrs);
            let form = self.wires.form(id, attrs, &derives);
            let form = match form {
                None if self.fields_without_form => Some(WireForm::Derive),
                form => form,
            };
            let body = match form {
                None | Some(WireForm::Manual) => continue,
                Some(WireForm::Via(ty, module)) => {
                    Body::Via(self.shape(index, module, &ty, &names, 0))
                }
                Some(WireForm::Fields(fields, module)) => Body::Record(
                    fields
                        .iter()
                        .map(|(name, ty)| (name.clone(), self.shape(index, module, ty, &names, 0)))
                        .collect(),
                ),
                Some(WireForm::Derive) => match &item {
                    Item::Struct(s) => self.fields(index, id.module, &s.fields, &names),
                    Item::Enum(e) => Body::Enum(
                        e.variants
                            .iter()
                            .map(|v| {
                                (
                                    v.ident.to_string(),
                                    self.fields(index, id.module, &v.fields, &names),
                                )
                            })
                            .collect(),
                    ),
                    Item::Union(u) => {
                        let fields = syn::Fields::Named(u.fields.clone());
                        self.fields(index, id.module, &fields, &names)
                    }
                    _ => continue,
                },
            };
            self.table.insert(
                key,
                Definition {
                    params: names.len(),
                    body,
                },
            );
        }
    }

    /// A type the description does not look into, as the surface writes it.
    fn opaque(
        &self,
        index: &mut Index<'_>,
        module: usize,
        ty: &Type,
        params: &[GenericName],
    ) -> Shape {
        self.wires.opaque(index, module, ty, params)
    }

    fn fields(
        &mut self,
        index: &mut Index<'_>,
        module: usize,
        fields: &syn::Fields,
        params: &[GenericName],
    ) -> Body {
        match fields {
            syn::Fields::Named(named) => Body::Record(
                named
                    .named
                    .iter()
                    .map(|f| {
                        (
                            f.ident
                                .as_ref()
                                .map(ToString::to_string)
                                .unwrap_or_default(),
                            self.shape(index, module, &f.ty, params, 0),
                        )
                    })
                    .collect(),
            ),
            syn::Fields::Unnamed(unnamed) => Body::Tuple(
                unnamed
                    .unnamed
                    .iter()
                    .map(|f| self.shape(index, module, &f.ty, params, 0))
                    .collect(),
            ),
            syn::Fields::Unit => Body::Unit,
        }
    }

    /// The layout description of `ty`, written in `module` with type parameters `params`.
    /// `depth` bounds alias expansion.
    pub fn shape(
        &mut self,
        index: &mut Index<'_>,
        module: usize,
        ty: &Type,
        params: &[GenericName],
        depth: usize,
    ) -> Shape {
        let boxed = |s: Shape| Box::new(s);
        match ty {
            Type::Paren(inner) => self.shape(index, module, &inner.elem, params, depth),
            Type::Group(inner) => self.shape(index, module, &inner.elem, params, depth),
            Type::Reference(r) => match &*r.elem {
                Type::Path(p) if p.path.is_ident("str") => Shape::Scalar(Scalar::Str),
                elem => self.shape(index, module, elem, params, depth),
            },
            Type::Slice(s) => Shape::List(boxed(self.shape(index, module, &s.elem, params, depth))),
            Type::Array(a) => match &a.len {
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(n),
                    ..
                }) => match n.base10_parse::<u64>() {
                    Ok(n) => {
                        Shape::Array(boxed(self.shape(index, module, &a.elem, params, depth)), n)
                    }
                    Err(_) => self.opaque(index, module, ty, params),
                },
                _ => self.opaque(index, module, ty, params),
            },
            Type::Tuple(t) if t.elems.is_empty() => Shape::Scalar(Scalar::Unit),
            Type::Tuple(t) => Shape::Tuple(
                t.elems
                    .iter()
                    .map(|e| self.shape(index, module, e, params, depth))
                    .collect(),
            ),
            Type::Path(p) if p.qself.is_none() => {
                self.path(index, module, ty, &p.path, params, depth)
            }
            _ => self.opaque(index, module, ty, params),
        }
    }

    fn path(
        &mut self,
        index: &mut Index<'_>,
        module: usize,
        ty: &Type,
        path: &syn::Path,
        params: &[GenericName],
        depth: usize,
    ) -> Shape {
        let opaque = self.opaque(index, module, ty, params);
        let Some(last) = path.segments.last() else {
            return opaque;
        };
        let name = last.ident.to_string();
        let args: Vec<Type> = match &last.arguments {
            syn::PathArguments::AngleBracketed(a) => a
                .args
                .iter()
                .filter_map(|arg| match arg {
                    syn::GenericArgument::Type(t) => Some(t.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        if path.segments.len() == 1
            && let Some(position) = params.iter().position(|p| p.name == name)
        {
            return Shape::Param(position);
        }
        let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
        let resolved = index.resolve_path(module, &segments);
        if let Ok(Found::Item(id)) = resolved {
            match index.item(id).clone() {
                Item::Struct(_) | Item::Enum(_) | Item::Union(_) => {
                    let key = self.key(index, id);
                    let args = args
                        .iter()
                        .map(|a| self.shape(index, module, a, params, depth))
                        .collect();
                    return Shape::Named(key, args);
                }
                Item::Type(alias) if depth < 16 => {
                    // An alias is its target, its parameters replaced by the arguments.
                    let names = param_names(&alias.generics);
                    let mut target = (*alias.ty).clone();
                    let mut substitute = Substitute {
                        names: &names,
                        args: &args,
                    };
                    substitute.visit_type_mut(&mut target);
                    let _ = substitute;
                    return self.shape(index, id.module, &target, params, depth + 1);
                }
                _ => return opaque,
            }
        }
        let one = |this: &mut Self, index: &mut Index<'_>, i: usize| {
            args.get(i)
                .map(|a| this.shape(index, module, a, params, depth))
                .unwrap_or(Shape::Opaque(String::new()))
        };
        let scalar = match name.as_str() {
            "bool" => Some(Scalar::Bool),
            "char" => Some(Scalar::Char),
            "u8" => Some(Scalar::U8),
            "u16" => Some(Scalar::U16),
            "u32" => Some(Scalar::U32),
            "u64" => Some(Scalar::U64),
            "u128" => Some(Scalar::U128),
            "usize" => Some(Scalar::Usize),
            "i8" => Some(Scalar::I8),
            "i16" => Some(Scalar::I16),
            "i32" => Some(Scalar::I32),
            "i64" => Some(Scalar::I64),
            "i128" => Some(Scalar::I128),
            "isize" => Some(Scalar::Isize),
            "f32" => Some(Scalar::F32),
            "f64" => Some(Scalar::F64),
            "str" | "String" | "Cow" => Some(Scalar::Str),
            "Duration" => Some(Scalar::Duration),
            _ => None,
        };
        if let Some(scalar) = scalar {
            if name == "Cow"
                && !matches!(args.first(), Some(Type::Path(p)) if p.path.is_ident("str"))
            {
                return one(self, index, 0);
            }
            return Shape::Scalar(scalar);
        }
        match name.as_str() {
            "Vec" | "VecDeque" => Shape::List(Box::new(one(self, index, 0))),
            "HashSet" | "BTreeSet" | "IndexSet" => Shape::Set(Box::new(one(self, index, 0))),
            "HashMap" | "BTreeMap" | "IndexMap" => {
                Shape::Map(Box::new(one(self, index, 0)), Box::new(one(self, index, 1)))
            }
            "Option" => Shape::Option(Box::new(one(self, index, 0))),
            "Result" if args.len() == 2 => {
                Shape::Result(Box::new(one(self, index, 0)), Box::new(one(self, index, 1)))
            }
            "Box" | "Arc" | "Rc" => match args.first() {
                Some(Type::Path(p)) if p.path.is_ident("str") => Shape::Scalar(Scalar::Str),
                Some(Type::Slice(s)) => {
                    Shape::List(Box::new(self.shape(index, module, &s.elem, params, depth)))
                }
                Some(Type::TraitObject(_)) | None => opaque,
                Some(_) => one(self, index, 0),
            },
            _ => opaque,
        }
    }
}

/// Replaces an alias's type parameters by its arguments.
struct Substitute<'s> {
    names: &'s [GenericName],
    args: &'s [Type],
}

impl VisitMut for Substitute<'_> {
    fn visit_type_mut(&mut self, ty: &mut Type) {
        if let Type::Path(p) = ty
            && p.qself.is_none()
            && p.path.segments.len() == 1
            && let Some(i) = self.names.iter().position(|n| p.path.is_ident(&n.name))
            && let Some(arg) = self.args.get(i)
        {
            *ty = arg.clone();
            return;
        }
        syn::visit_mut::visit_type_mut(self, ty);
    }
}
