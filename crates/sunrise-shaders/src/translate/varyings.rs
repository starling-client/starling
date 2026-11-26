//! The varyings between a program's stages: which travel, at which location, and in what parts.

use std::collections::BTreeMap;

use super::Error;
use super::scan::Stage;

/// The varyings of a program: by name, its location and declaration (`@` for the name); the
/// matrices, arrays and structs that travel in parts (their global, and each part with how to
/// reach it); and the vertex outputs that stay globals of the vertex stage.
pub(super) struct Varyings {
    pub locations: BTreeMap<String, (u32, String)>,
    pub split: BTreeMap<String, (String, Vec<(String, String)>)>,
    pub private: Vec<String>,
}

/// Varyings by name: the vertex outputs the fragment stage reads, then fragment inputs the vertex
/// stage lacks. A matrix or array travels as one varying per column or element
/// (`varying_parts`), a struct as its members packed into vectors (`struct_parts`). Each takes
/// one location of the `max_locations` the device gives a stage interface
/// (`max_inter_stage_shader_variables`); a varying past them is refused at its line before its
/// parts are made, as the device could never build the pipeline.
pub(super) fn assign(
    v: &Stage,
    f: &Stage,
    errors: &mut Vec<Error>,
    max_locations: usize,
) -> Varyings {
    let mut locations: BTreeMap<String, (u32, String)> = BTreeMap::new();
    let mut split: BTreeMap<String, (String, Vec<(String, String)>)> = BTreeMap::new();
    let mut private = Vec::new();
    let vars = v.outs.iter().map(|o| (v, o));
    for (stage, var) in vars.chain(f.ins.iter().map(|i| (f, i))) {
        if locations.contains_key(&var.name) || split.contains_key(&var.name) {
            continue;
        }
        // A pack's varying its fragment stage never reads takes no location, as GL's linker
        // drops an inactive varying: a vertex output stays a global of the vertex stage (a
        // struct's already does), so a pack writing more than it reads keeps within the GPU's
        // varyings.
        // Ours (`hz_FogFragCoord` and the other built-ins, `sunrisePatchPos`) are read by code
        // of ours, not the pack's: they always travel.
        let ours = var.name.starts_with("hz_") || var.name.starts_with("sunrise");
        if !ours && !reads(f, &var.name) {
            if std::ptr::eq(stage, v) && !var.structure {
                private.push(plain(&var.decl).replace('@', &var.name));
            }
            continue;
        }
        // This one gets the locations the earlier ones left, and is refused before its parts are
        // made when it needs more.
        let room = max_locations.saturating_sub(locations.len());
        let too_many = || {
            stage.error(
                var.at,
                &format!(
                    "`{}`: a program's varyings take at most {max_locations} locations",
                    var.name
                ),
            )
        };
        if var.structure {
            match struct_parts(&var.decl, &stage.structs, room) {
                Ok(Some(parts)) => {
                    let mut copies = Vec::new();
                    for (k, (decl, members)) in parts.into_iter().enumerate() {
                        let part = format!("hz_v_{}_{k}", var.name);
                        let next = locations.len() as u32;
                        locations.insert(part.clone(), (next, decl));
                        for (swizzle, access) in members {
                            copies.push((format!("{part}{swizzle}"), access));
                        }
                    }
                    split.insert(var.name.clone(), (String::new(), copies));
                }
                Ok(None) => errors.push(Error::new(
                    stage.pp.file(),
                    0,
                    format!("`{}`: this struct can't be a varying", var.name),
                )),
                Err(TooManyParts) => errors.push(too_many()),
            }
            continue;
        }
        match varying_parts(&var.decl, room) {
            Ok(Some((global, parts))) => {
                let mut copies = Vec::new();
                for (k, (decl, access)) in parts.into_iter().enumerate() {
                    let part = format!("hz_v_{}_{k}", var.name);
                    let next = locations.len() as u32;
                    locations.insert(part.clone(), (next, decl));
                    copies.push((part, access));
                }
                split.insert(var.name.clone(), (global, copies));
            }
            Ok(None) if room > 0 => {
                let next = locations.len() as u32;
                locations.insert(var.name.clone(), (next, var.decl.clone()));
            }
            Ok(None) | Err(TooManyParts) => errors.push(too_many()),
        }
    }
    Varyings {
        locations,
        split,
        private,
    }
}

/// Whether `stage`'s code names `name` outside the declarations moved into the header.
fn reads(stage: &Stage, name: &str) -> bool {
    (stage.tokens.iter().enumerate())
        .any(|(i, t)| *t == name && !stage.removed.iter().any(|r| r.contains(&i)))
}

/// `decl` without its interpolation qualifiers: a plain global's declaration.
fn plain(decl: &str) -> String {
    decl.split(' ')
        .filter(|w| {
            !matches!(
                *w,
                "flat" | "smooth" | "noperspective" | "centroid" | "sample"
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A varying split into more parts than the locations the program has left: refused before a
/// part is made for each.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct TooManyParts;

/// A declaration and what it holds: for a split varying, its private global and each part's
/// declaration with how to reach it; for a part of a struct varying, its declaration and each
/// member's swizzle with how to reach the member.
type Split = (String, Vec<(String, String)>);

/// A matrix or array varying (`flat mat3 @[2];`): wgpu's stage interfaces take only scalars
/// and vectors (WGSL), so it becomes a private global (`mat3 @[2];`) and one varying per
/// column or element, each with the qualifiers and how to reach it (`[1][2]`). `None` for
/// scalars and vectors, and for sizes that are not plain numbers; [`TooManyParts`] for more
/// parts than `room`, found before any part is made.
pub(super) fn varying_parts(decl: &str, room: usize) -> Result<Option<Split>, TooManyParts> {
    let shape = || {
        let (before, after) = decl.split_once('@')?;
        let mut words: Vec<&str> = before.split_whitespace().collect();
        let ty = words.pop()?;
        let qualifiers: String = words.iter().map(|w| format!("{w} ")).collect();
        let array = after.trim().trim_end_matches(';').trim();
        let mut sizes = Vec::new();
        let mut rest = array;
        while let Some(r) = rest.strip_prefix('[') {
            let (n, r) = r.split_once(']')?;
            sizes.push(n.trim().parse::<usize>().ok()?);
            rest = r.trim_start();
        }
        if !rest.is_empty() {
            return None;
        }
        // matC or matCxR: C columns of R rows.
        let column = ty.strip_prefix("mat").and_then(|d| {
            let (c, r) = d.split_once('x').unwrap_or((d, d));
            Some((c.parse::<usize>().ok()?, r.parse::<usize>().ok()?))
        });
        if sizes.is_empty() && column.is_none() {
            return None;
        }
        let part_ty = column.map_or(ty.to_owned(), |(_, r)| format!("vec{r}"));
        if let Some((c, _)) = column {
            sizes.push(c);
        }
        Some((ty, qualifiers, array, sizes, part_ty))
    };
    let Some((ty, qualifiers, array, sizes, part_ty)) = shape() else {
        return Ok(None);
    };
    let count = (sizes.iter())
        .try_fold(1usize, |count, &size| count.checked_mul(size))
        .filter(|&count| count <= room)
        .ok_or(TooManyParts)?;
    let parts = (0..count)
        .map(|mut k| {
            let mut access = String::new();
            for &size in sizes.iter().rev() {
                access.insert_str(0, &format!("[{}]", k % size));
                k /= size;
            }
            (format!("{qualifiers}{part_ty} @;"), access)
        })
        .collect();
    Ok(Some((format!("{ty} @{array};"), parts)))
}

/// A struct varying (`flat Params @;`) as vectors: its scalar, vector and matrix members, through
/// nested structs and arrays, packed four components at a time by base type. Each part: its
/// declaration and, per member, the swizzle of the part and how to reach the member (`.a[1].b`).
/// `None` for a member that can't travel (`bool`, an unknown type, a size that is not a number);
/// [`TooManyParts`] for more parts than `room`, found before more than four leaves a part are
/// listed.
pub(super) fn struct_parts(
    decl: &str,
    structs: &std::collections::HashMap<String, Vec<(String, String)>>,
    room: usize,
) -> Result<Option<Vec<Split>>, TooManyParts> {
    let Some((before, after)) = decl.split_once('@') else {
        return Ok(None);
    };
    let mut words: Vec<&str> = before.split_whitespace().collect();
    let Some(ty) = words.pop() else {
        return Ok(None);
    };
    let qualifiers: String = words.iter().map(|w| format!("{w} ")).collect();
    let mut leaves = Vec::new();
    // A part holds at most four leaves (each a component or more), so more than `4 * room`
    // leaves take more than `room` parts. A leaf's path takes a step for the varying, each array
    // dimension and each member it goes through: without a struct inside itself, at most
    // `depth` steps, the varying's and every member's dimensions plus every struct. A longer path
    // is a struct inside itself, which can't travel; and `4 * room` leaves take at most
    // `4 * room * depth` steps, so past them (elements without leaves) the varying takes more
    // parts than the room.
    let dimensions = |array: &str| array.matches('[').count();
    let depth = 1
        + structs.len()
        + dimensions(after)
        + (structs.values().flatten())
            .map(|(_, member)| dimensions(member))
            .sum::<usize>();
    let mut budget = Budget {
        leaves: 4 * room,
        steps: (4 * room).saturating_mul(depth),
        depth,
        level: 0,
    };
    match leaves_of(
        ty,
        after.trim().trim_end_matches(';'),
        String::new(),
        structs,
        &mut leaves,
        &mut budget,
    ) {
        Ok(()) => {}
        Err(Leaves::CantTravel) => return Ok(None),
        Err(Leaves::TooMany) => return Err(TooManyParts),
    }
    // Slots of one base type and up to four components: (base, used, members (from, size, access)).
    let mut slots: Vec<(&str, usize, Vec<(usize, usize, String)>)> = Vec::new();
    for (base, size, access) in leaves {
        match slots
            .iter_mut()
            .find(|(b, used, _)| *b == base && used + size <= 4)
        {
            Some(slot) => {
                slot.2.push((slot.1, size, access));
                slot.1 += size;
            }
            None => slots.push((base, size, vec![(0, size, access)])),
        }
    }
    if slots.len() > room {
        return Err(TooManyParts);
    }
    Ok(Some(
        slots
            .into_iter()
            .map(|(base, used, members)| {
                let ty = match (base, used) {
                    (b, 1) => b.to_owned(),
                    ("float", n) => format!("vec{n}"),
                    ("int", n) => format!("ivec{n}"),
                    (_, n) => format!("uvec{n}"),
                };
                // Integers only travel flat.
                let flat = if base != "float" && !qualifiers.contains("flat") {
                    "flat "
                } else {
                    ""
                };
                let members = members
                    .into_iter()
                    .map(|(from, size, access)| {
                        let swizzle = if used == 1 {
                            String::new()
                        } else {
                            format!(".{}", &"xyzw"[from..from + size])
                        };
                        (swizzle, access)
                    })
                    .collect();
                (format!("{flat}{qualifiers}{ty} @;"), members)
            })
            .collect(),
    ))
}

/// Why [`leaves_of`] stopped.
enum Leaves {
    /// A member that can't travel.
    CantTravel,
    /// More steps than `steps` allowed.
    TooMany,
}

/// What [`leaves_of`] may still list before the varying takes more parts than it has room for.
struct Budget {
    leaves: usize,
    steps: usize,
    /// The longest path a leaf can take, and the path's length so far: longer, the varying is a
    /// struct inside itself, which can't travel.
    depth: usize,
    level: usize,
}

impl Budget {
    fn step(&mut self) -> Result<(), Leaves> {
        self.steps = self.steps.checked_sub(1).ok_or(Leaves::TooMany)?;
        Ok(())
    }

    fn leaf(&mut self) -> Result<(), Leaves> {
        self.leaves = self.leaves.checked_sub(1).ok_or(Leaves::TooMany)?;
        self.step()
    }
}

/// The scalars and vectors inside a value of type `ty` with array suffix `array`, reached by
/// `access`: (base type, components, access). Each element and member takes a step of `budget`,
/// each leaf a leaf and a step.
fn leaves_of(
    ty: &str,
    array: &str,
    access: String,
    structs: &std::collections::HashMap<String, Vec<(String, String)>>,
    out: &mut Vec<(&'static str, usize, String)>,
    budget: &mut Budget,
) -> Result<(), Leaves> {
    budget.step()?;
    if budget.level >= budget.depth {
        return Err(Leaves::CantTravel);
    }
    budget.level += 1;
    let listed = leaves_in(ty, array, access, structs, out, budget);
    budget.level -= 1;
    listed
}

/// [`leaves_of`] one step down the path.
fn leaves_in(
    ty: &str,
    array: &str,
    access: String,
    structs: &std::collections::HashMap<String, Vec<(String, String)>>,
    out: &mut Vec<(&'static str, usize, String)>,
    budget: &mut Budget,
) -> Result<(), Leaves> {
    if let Some(rest) = array.trim().strip_prefix('[') {
        let (n, rest) = rest.split_once(']').ok_or(Leaves::CantTravel)?;
        let n = n.trim().parse::<usize>().map_err(|_| Leaves::CantTravel)?;
        for k in 0..n {
            leaves_of(ty, rest, format!("{access}[{k}]"), structs, out, budget)?;
        }
        return Ok(());
    }
    if let Some(members) = structs.get(ty) {
        for (member_ty, member) in members {
            let (name, array) = member
                .split_once('[')
                .map_or((member.as_str(), ""), |(n, _)| (n, &member[n.len()..]));
            leaves_of(
                member_ty,
                array,
                format!("{access}.{name}"),
                structs,
                out,
                budget,
            )?;
        }
        return Ok(());
    }
    // A vector's or matrix's size GLSL has, 2 to 4: another (`vec5`) can't travel, and would
    // not fit a part's four components.
    let number = |n: &str| {
        n.parse::<usize>()
            .ok()
            .filter(|n| (2..=4).contains(n))
            .ok_or(Leaves::CantTravel)
    };
    if let Some(dims) = ty.strip_prefix("mat") {
        let (c, r) = dims.split_once('x').unwrap_or((dims, dims));
        let (c, r) = (number(c)?, number(r)?);
        for k in 0..c {
            budget.leaf()?;
            out.push(("float", r, format!("{access}[{k}]")));
        }
        return Ok(());
    }
    let (base, size) = match ty {
        "float" => ("float", 1),
        "int" => ("int", 1),
        "uint" => ("uint", 1),
        _ => {
            let (base, n) = if let Some(n) = ty.strip_prefix("vec") {
                ("float", n)
            } else if let Some(n) = ty.strip_prefix("ivec") {
                ("int", n)
            } else {
                ("uint", ty.strip_prefix("uvec").ok_or(Leaves::CantTravel)?)
            };
            (base, number(n)?)
        }
    };
    budget.leaf()?;
    out.push((base, size, access));
    Ok(())
}
