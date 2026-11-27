//! The check a pack passes before it runs: in the game when it loads (a pack
//! that fails keeps the last good one drawing), in the launcher before it
//! installs, in the registry's CI, and in `sunrise-shader check`.
//!
//! Each stage is built as the renderer builds it, parsed and validated by
//! naga, checked against its signature and the API's rules (no bindings, no
//! entry points, no `sunrise_` names; in 0.2 none of the structs Sunrise hands it
//! constructed), then translated to SPIR-V, Metal and HLSL with its options at their
//! defaults, minimums and maximums.
use crate::inputs::{DISPLACEMENT_INPUT, Frame};
use crate::manifest::{OptionKind, PackOption};
use crate::package::Pack;
use crate::stages::{StageKind, compose};
use naga::valid::ModuleInfo;
use std::collections::HashSet;
use sunrise_mod_pipeline::check::{Composed, Problem, located, problem, translate, validate};

/// naga expressions a stage may hold, over all its functions.
pub const MAX_EXPRESSIONS: usize = 20_000;
/// Texture reads (samples, gathers and loads) a stage may write.
pub const MAX_TEXTURE_READS: usize = 64;

/// One stage that passed, ready for the renderer.
#[derive(Debug)]
pub struct CheckedStage {
    pub kind: StageKind,
    pub file: String,
    /// The manifest's name for it, else its file.
    pub label: String,
    pub module: naga::Module,
    pub info: ModuleInfo,
    /// The stage's WGSL without the entry points (the prelude, the shared
    /// files, the file), for a renderer that calls the stage function from
    /// its own shader (the sky).
    pub source: String,
    pub expressions: usize,
    pub texture_reads: usize,
}

impl CheckedStage {
    /// Whether the stage's function, or a function it calls, reads the prelude's `global`
    /// (a resource of the stage, such as `reflection`).
    pub fn reads(&self, global: &str) -> bool {
        let Some((function, _)) = self
            .module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_deref() == Some(self.kind.function()))
        else {
            return false;
        };
        self.module
            .global_variables
            .iter()
            .filter(|(_, g)| g.name.as_deref() == Some(global))
            .any(|(handle, _)| !self.info[function][handle].is_empty())
    }
}

#[derive(Debug)]
pub struct Report {
    pub stages: Vec<CheckedStage>,
    /// The manifest's `[sky]`.
    pub sky: Option<crate::manifest::Stage>,
    /// The manifest's `[reflection]`.
    pub reflection: Option<crate::manifest::Reflection>,
    /// The manifest's `[shadow]`.
    pub shadow: Option<crate::manifest::Shadow>,
    /// The manifest's `[material] fog`: whether Sunrise fogs the material's colour.
    pub material_fog: bool,
    /// The manifest's `[[textures]]`, and each image decoded, in that order.
    pub textures: Vec<(crate::manifest::PackTexture, Image)>,
    /// The manifest the stages were checked against.
    pub manifest: crate::manifest::Manifest,
}

#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// RGBA, 8 bits a channel, rows from the top.
    pub rgba: Vec<u8>,
}

/// Decodes the PNG `bytes` into RGBA8.
pub fn decode_png(bytes: &[u8]) -> Result<Image, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let (width, height) = (reader.info().width, reader.info().height);
    let max = crate::manifest::MAX_TEXTURE_SIZE;
    if width == 0 || height == 0 || width > max || height > max {
        return Err(format!(
            "{width}×{height}; a texture is at most {max}×{max}"
        ));
    }
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("too large")?];
    let frame = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let data = &buffer[..frame.buffer_size()];
    let pixels = (width * height) as usize;
    let rgba = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data
            .chunks(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => data
            .chunks(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed colour was not expanded".into()),
    };
    if rgba.len() != pixels * 4 {
        return Err("unexpected pixel data".into());
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

/// Checks every stage of `pack`, and that each option is an `override` of
/// its type in some stage, or one `[shadow]` sizes its map by. Returns every problem
/// found, not just the first.
pub fn check(pack: &Pack) -> Result<Report, Vec<Problem>> {
    let m = &pack.manifest;
    let stages = m
        .sky
        .iter()
        .map(|s| (StageKind::Sky, s))
        .chain(m.vertex.iter().map(|s| (StageKind::Vertex, s)))
        .chain(m.material.iter().map(|s| (StageKind::Material, s)))
        .chain(m.weather.iter().map(|s| (StageKind::Weather, s)))
        .chain(m.post.iter().map(|s| (StageKind::Post, s)))
        .chain(m.tonemap.iter().map(|s| (StageKind::Tonemap, s)))
        .chain(m.entity.iter().map(|s| (StageKind::Entity, s)));
    let (mut checked, mut problems) = (Vec::new(), Vec::new());
    let mut post = 0;
    for (kind, stage) in stages {
        let entry = if kind == StageKind::Post { post } else { 0 };
        post += (kind == StageKind::Post) as usize;
        match check_stage(pack, kind, &stage.file, entry) {
            Ok(mut c) => {
                c.label = stage.name.clone().unwrap_or_else(|| stage.file.clone());
                checked.push(c);
            }
            Err(p) => problems.extend(p),
        }
    }
    // The options the shadow map's size follows need no `override`: the frame has the size.
    let sizes: Vec<&String> = m
        .shadow
        .iter()
        .flat_map(|s| s.resolution_option.iter().chain(&s.distance_option))
        .collect();
    if problems.is_empty() {
        for option in m.options.iter().filter(|o| !sizes.contains(&&o.id)) {
            let used = checked.iter().any(|c| {
                c.module
                    .overrides
                    .iter()
                    .any(|(_, o)| o.name.as_deref() == Some(&option.id))
            });
            if !used {
                problems.push(problem(
                    crate::MANIFEST,
                    format!(
                        "option {:?}: no stage declares `override {}: {}`",
                        option.id,
                        option.id,
                        option.kind.wgsl_type()
                    ),
                ));
            }
        }
    }
    let mut textures = Vec::new();
    for t in &m.textures {
        match decode_png(pack.source_bytes(&t.file).unwrap_or_default()) {
            Ok(image) => textures.push((t.clone(), image)),
            Err(e) => problems.push(problem(&t.file, e)),
        }
    }
    if problems.is_empty() {
        Ok(Report {
            sky: pack.manifest.sky.clone(),
            stages: checked,
            textures,
            reflection: m.reflection.clone(),
            shadow: m.shadow.clone(),
            material_fog: m.material.as_ref().and_then(|s| s.fog).unwrap_or(true),
            manifest: m.clone(),
        })
    } else {
        Err(problems)
    }
}

/// Builds and checks one stage of `pack` from its file, on a thread of its own with the stack
/// and the bounds of [`sunrise_mod_pipeline::guard`]: a pack's file never crashes the checker.
pub fn check_stage(
    pack: &Pack,
    kind: StageKind,
    file: &str,
    entry: usize,
) -> Result<CheckedStage, Vec<Problem>> {
    sunrise_mod_pipeline::guard::guarded(|| build_stage(pack, kind, file, entry))
        .unwrap_or_else(|reason| Err(vec![problem(file, reason)]))
}

fn build_stage(
    pack: &Pack,
    kind: StageKind,
    file: &str,
    entry: usize,
) -> Result<CheckedStage, Vec<Problem>> {
    // First without the entry points, so a missing or mistyped stage function
    // is reported as such rather than as a call that doesn't resolve.
    let mut own = compose(pack, kind, file, entry);
    if let Some((at, message)) = sunrise_mod_pipeline::guard::refusal(&own.text) {
        return Err(vec![located(&own, at, message)]);
    }
    let entries = own.parts.pop().expect("entries are the last part");
    own.text.truncate(entries.1.start);
    // The material prelude's LabPBR functions call into the renderer: a stand-in here.
    if kind == StageKind::Material {
        own.text += crate::stages::PBR_STUB;
    }
    let module = parse(&own)?;
    own.text.truncate(entries.1.start);
    let mut problems = signature(&module, kind, file);
    if kind == StageKind::Material && pack.manifest.displacement.is_some() {
        problems.extend(function_signature(
            &module,
            "displacement",
            &["DisplacementInput"],
            "f32",
            crate::stages::DISPLACEMENT_SIGNATURE,
            file,
        ));
    }
    problems.extend(api_rules(&module, kind, &own));
    // 0.1's structs won't grow: it is retired when 0.2 is on by default.
    if pack.manifest.api == crate::Api::V0_2 {
        problems.extend(constructions(&module, &own));
    }
    problems.extend(options(&module, &pack.manifest.options, file));
    if !problems.is_empty() {
        return Err(problems);
    }
    let composed = compose(pack, kind, file, entry);
    let module = parse(&composed)?;
    let info = validate(&composed, &module)?;
    let (expressions, texture_reads) = cost(&module);
    let mut problems = Vec::new();
    let engine = kind == StageKind::Material && pack.manifest.coverage == crate::Coverage::Engine;
    if engine && let Some(at) = reachable_discard(&module, kind.function()) {
        problems.push(located(
                &composed,
                at.to_range().map(|r| r.start),
                "`discard` reachable from `material`: under `[material] coverage = \"engine\"` (the \
                 default) Sunrise's alpha test on the texture decides what is drawn; a pack that cuts \
                 its own holes sets `coverage = \"pack\"`"
                    .into(),
            ));
    }
    problems.extend(cost_problems((expressions, texture_reads), file, "stage"));
    problems.extend(translate_options(
        &module,
        &info,
        &pack.manifest.options,
        file,
    ));
    if !problems.is_empty() {
        return Err(problems);
    }
    Ok(CheckedStage {
        kind,
        file: file.to_owned(),
        label: file.to_owned(),
        module,
        info,
        source: own.text,
        expressions,
        texture_reads,
    })
}

/// A refusal for each of `cost`'s counts over its cap; `what` names what holds them.
pub(crate) fn cost_problems(
    (expressions, texture_reads): (usize, usize),
    file: &str,
    what: &str,
) -> Vec<Problem> {
    let mut problems = Vec::new();
    if expressions > MAX_EXPRESSIONS {
        problems.push(problem(
            file,
            format!("{expressions} expressions; a {what} may hold {MAX_EXPRESSIONS}"),
        ));
    }
    if texture_reads > MAX_TEXTURE_READS {
        problems.push(problem(
            file,
            format!("{texture_reads} texture reads; a {what} may write {MAX_TEXTURE_READS}"),
        ));
    }
    problems
}

fn parse(composed: &Composed) -> Result<naga::Module, Vec<Problem>> {
    naga::front::wgsl::parse_str(&composed.text).map_err(|e| {
        let label = e.labels().next();
        let mut message = e.message().to_owned();
        if let Some((_, text)) = label.filter(|(_, t)| !t.is_empty() && !message.contains(*t)) {
            message += &format!(" ({text})");
        }
        let at = label.and_then(|(span, _)| span.to_range()).map(|r| r.start);
        vec![located(composed, at, message)]
    })
}

fn type_name(module: &naga::Module, ty: naga::Handle<naga::Type>) -> String {
    let t = &module.types[ty];
    if let Some(name) = &t.name {
        return name.clone();
    }
    match t.inner {
        naga::TypeInner::Scalar(s) => scalar_name(s).to_owned(),
        naga::TypeInner::Vector { size, scalar } => {
            format!("vec{}<{}>", size as u8, scalar_name(scalar))
        }
        _ => format!("{:?}", t.inner),
    }
}

fn scalar_name(s: naga::Scalar) -> &'static str {
    match (s.kind, s.width) {
        (naga::ScalarKind::Bool, _) => "bool",
        (naga::ScalarKind::Float, 4) => "f32",
        (naga::ScalarKind::Sint, 4) => "i32",
        (naga::ScalarKind::Uint, 4) => "u32",
        _ => "another scalar",
    }
}

fn signature(module: &naga::Module, kind: StageKind, file: &str) -> Vec<Problem> {
    let found = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some(kind.function()));
    let Some((_, f)) = found else {
        return vec![problem(file, format!("defines no `{}`", kind.signature()))];
    };
    let args: Vec<String> = f
        .arguments
        .iter()
        .map(|a| type_name(module, a.ty))
        .collect();
    let result = f.result.as_ref().map(|r| type_name(module, r.ty));
    if args == [kind.input().name] && result.as_deref() == Some(kind.output()) {
        return Vec::new();
    }
    vec![problem(
        file,
        format!(
            "`fn {}({}) -> {}` should be `{}`",
            kind.function(),
            args.join(", "),
            result.as_deref().unwrap_or("()"),
            kind.signature()
        ),
    )]
}

/// The module defines `fn <name>(<args>) -> <output>`, written out as `expected`.
fn function_signature(
    module: &naga::Module,
    name: &str,
    args_expected: &[&str],
    output: &str,
    expected: &str,
    file: &str,
) -> Vec<Problem> {
    let found = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some(name));
    let Some((_, f)) = found else {
        return vec![problem(file, format!("defines no `{expected}`"))];
    };
    let args: Vec<String> = f
        .arguments
        .iter()
        .map(|a| type_name(module, a.ty))
        .collect();
    let result = f.result.as_ref().map(|r| type_name(module, r.ty));
    if args == args_expected && result.as_deref() == Some(output) {
        return Vec::new();
    }
    vec![problem(
        file,
        format!(
            "`fn {name}({}) -> {}` should be `{expected}`",
            args.join(", "),
            result.as_deref().unwrap_or("()"),
        ),
    )]
}

/// The names of everything a module defines at its top level.
fn names(module: &naga::Module) -> HashSet<String> {
    let types = module.types.iter().filter_map(|(_, t)| t.name.clone());
    let functions = module.functions.iter().filter_map(|(_, f)| f.name.clone());
    let globals = module
        .global_variables
        .iter()
        .filter_map(|(_, g)| g.name.clone());
    let constants = module.constants.iter().filter_map(|(_, c)| c.name.clone());
    let overrides = module.overrides.iter().filter_map(|(_, o)| o.name.clone());
    types
        .chain(functions)
        .chain(globals)
        .chain(constants)
        .chain(overrides)
        .collect()
}

/// A pack's files declare no bindings, no entry points and no `sunrise_`
/// names: what the prelude doesn't define is the pack's.
fn api_rules(module: &naga::Module, kind: StageKind, own: &Composed) -> Vec<Problem> {
    // The prelude as composed: the stage's own, with the pack's `[[textures]]`. The material
    // prelude's LabPBR functions call into the renderer: its stand-in is ours too.
    let mut text = own.text[own.parts[0].1.clone()].to_owned();
    if kind == StageKind::Material {
        text += crate::stages::PBR_STUB;
    }
    let prelude = naga::front::wgsl::parse_str(&text).expect("the prelude parses");
    let ours = names(&prelude);
    let file = &own.parts[own.parts.len() - 1].0;
    let mut problems = Vec::new();
    for e in &module.entry_points {
        problems.push(problem(
            file,
            format!(
                "declares the entry point `{}`: a stage is a function, Sunrise adds the entry points",
                e.name
            ),
        ));
    }
    for (_, g) in module.global_variables.iter() {
        let name = g.name.clone().unwrap_or_default();
        if g.binding.is_some() && !ours.contains(&name) {
            problems.push(problem(
                file,
                format!(
                    "declares the binding `{name}`: a stage reads only the resources of its prelude"
                ),
            ));
        }
    }
    let mut theirs: Vec<String> = names(module)
        .into_iter()
        .filter(|n| !ours.contains(n) && n.starts_with("sunrise_"))
        .collect();
    theirs.sort();
    for name in theirs {
        problems.push(problem(
            file,
            format!("`{name}`: names starting with sunrise_ are Sunrise's"),
        ));
    }
    problems
}

/// 0.2: the pack's files construct none of the structs Sunrise hands a stage (`Frame`, the
/// stage inputs and the post helpers' `SurfaceData`). They may gain fields, which would break a
/// positional `PostInput(...)`; reading them and passing them along stay as they are. `Surface`,
/// which `material` returns, is built.
fn constructions(module: &naga::Module, own: &Composed) -> Vec<Problem> {
    let provided: Vec<&str> = StageKind::ALL
        .iter()
        .map(|k| k.input().name)
        .chain([Frame::WGSL.name, DISPLACEMENT_INPUT.name, "SurfaceData"])
        .collect();
    let arenas = module
        .functions
        .iter()
        .map(|(_, f)| &f.expressions)
        .chain([&module.global_expressions]);
    let in_pack = |at: Option<usize>| {
        at.is_some_and(|at| own.parts[1..].iter().any(|(_, r)| r.contains(&at)))
    };
    let mut problems = Vec::new();
    for arena in arenas {
        for (handle, e) in arena.iter() {
            let naga::Expression::Compose { ty, .. } = *e else {
                continue;
            };
            let Some(name) = module.types[ty].name.as_deref() else {
                continue;
            };
            let at = arena.get_span(handle).to_range().map(|r| r.start);
            // Built in the pack's files (the prelude's helpers aren't the pack's), as a struct the
            // prelude declares (a pack's own struct of the same name is its own).
            let sunrise_s = (module.types.get_span(ty).to_range())
                .is_some_and(|r| own.parts[0].1.contains(&r.start));
            if provided.contains(&name) && sunrise_s && in_pack(at) {
                problems.push(located(
                    own,
                    at,
                    format!("constructs `{name}`: it is Sunrise's and may gain fields; read it, don't construct it"),
                ));
            }
        }
    }
    problems
}

/// Where a `discard` is that the function `name` can reach: in its own body, its nested blocks,
/// or a function it calls, however deep.
fn reachable_discard(module: &naga::Module, name: &str) -> Option<naga::Span> {
    fn walk(
        block: &naga::Block,
        calls: &mut Vec<naga::Handle<naga::Function>>,
    ) -> Option<naga::Span> {
        for (statement, span) in block.span_iter() {
            let found = match statement {
                naga::Statement::Kill => return Some(*span),
                naga::Statement::Block(b) => walk(b, calls),
                naga::Statement::If { accept, reject, .. } => {
                    walk(accept, calls).or_else(|| walk(reject, calls))
                }
                naga::Statement::Switch { cases, .. } => {
                    cases.iter().find_map(|c| walk(&c.body, calls))
                }
                naga::Statement::Loop {
                    body, continuing, ..
                } => walk(body, calls).or_else(|| walk(continuing, calls)),
                naga::Statement::Call { function, .. } => {
                    calls.push(*function);
                    None
                }
                _ => None,
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }
    let (start, _) = module
        .functions
        .iter()
        .find(|(_, f)| f.name.as_deref() == Some(name))?;
    let (mut calls, mut seen) = (vec![start], HashSet::new());
    while let Some(f) = calls.pop() {
        if seen.insert(f)
            && let Some(span) = walk(&module.functions[f].body, &mut calls)
        {
            return Some(span);
        }
    }
    None
}

/// Each `override` is an option of its type, or has a default of its own.
fn options(module: &naga::Module, options: &[PackOption], file: &str) -> Vec<Problem> {
    let mut problems = Vec::new();
    for (_, o) in module.overrides.iter() {
        let name = o.name.as_deref().unwrap_or("?");
        let ty = type_name(module, o.ty);
        match options.iter().find(|opt| opt.id == name) {
            Some(opt) if ty != opt.kind.wgsl_type() => problems.push(problem(
                file,
                format!(
                    "`override {name}: {ty}`: option {name:?} is a {}, so its override is `{}`",
                    kind_name(&opt.kind),
                    opt.kind.wgsl_type()
                ),
            )),
            None if o.init.is_none() => problems.push(problem(
                file,
                format!("`override {name}` has no default and no [[options]] entry"),
            )),
            _ => {}
        }
    }
    problems
}

fn kind_name(kind: &OptionKind) -> &'static str {
    match kind {
        OptionKind::Bool { .. } => "bool",
        OptionKind::Float { .. } => "float",
        OptionKind::Int { .. } => "int",
        OptionKind::Choice { .. } => "choice",
    }
}

pub(crate) fn cost(module: &naga::Module) -> (usize, usize) {
    let functions = module
        .functions
        .iter()
        .map(|(_, f)| f)
        .chain(module.entry_points.iter().map(|e| &e.function));
    let (mut expressions, mut reads) = (0, 0);
    for f in functions {
        expressions += f.expressions.len();
        reads += f
            .expressions
            .iter()
            .filter(|(_, e)| {
                matches!(
                    e,
                    naga::Expression::ImageSample { .. } | naga::Expression::ImageLoad { .. }
                )
            })
            .count();
    }
    (expressions, reads)
}

/// The option values a pack is translated with: its defaults, then every
/// option at its minimum, then at its maximum.
fn option_sets(options: &[PackOption]) -> [(&'static str, Vec<(String, f64)>); 3] {
    let at = |pick: fn(&OptionKind) -> f64| {
        options
            .iter()
            .map(|o| (o.id.clone(), pick(&o.kind)))
            .collect()
    };
    [
        ("defaults", at(OptionKind::default_constant)),
        (
            "minimums",
            at(|k| match *k {
                OptionKind::Bool { .. } | OptionKind::Choice { .. } => 0.0,
                OptionKind::Float { min, .. } => f64::from(min),
                OptionKind::Int { min, .. } => f64::from(min),
            }),
        ),
        (
            "maximums",
            at(|k| match k {
                OptionKind::Bool { .. } => 1.0,
                OptionKind::Choice { values, .. } => (values.len() - 1) as f64,
                OptionKind::Float { max, .. } => f64::from(*max),
                OptionKind::Int { max, .. } => f64::from(*max),
            }),
        ),
    ]
}

/// Translates a stage with its options at their defaults, then minimums, then maximums, as wgpu
/// does when it builds the pipelines: the refusals of the first set refused, each naming the set
/// and the backend.
fn translate_options(
    module: &naga::Module,
    info: &ModuleInfo,
    options: &[PackOption],
    file: &str,
) -> Vec<Problem> {
    let present: HashSet<&str> = module
        .overrides
        .iter()
        .filter_map(|(_, o)| o.name.as_deref())
        .collect();
    for (set, values) in option_sets(options) {
        let constants = values
            .into_iter()
            .filter(|(id, _)| present.contains(id.as_str()))
            .collect();
        let refusals = translate(module, info, &constants);
        if !refusals.is_empty() {
            return refusals
                .into_iter()
                .map(|refusal| problem(file, format!("with the options at their {set}, {refusal}")))
                .collect();
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cap is on the count itself: 20,000 expressions pass, one more is refused.
    #[test]
    fn the_expression_cap_is_exact() {
        let file = "shaders/post.wgsl";
        assert!(cost_problems((20_000, 64), file, "stage").is_empty());
        let over = cost_problems((20_001, 65), file, "stage");
        assert_eq!(
            over.iter().map(Problem::to_string).collect::<Vec<_>>(),
            [
                format!("{file}: 20001 expressions; a stage may hold 20000"),
                format!("{file}: 65 texture reads; a stage may write 64"),
            ]
        );
    }
}
