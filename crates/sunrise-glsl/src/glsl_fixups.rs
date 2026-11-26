//! GLSL that vanilla's GL compiler takes and naga's front end doesn't (#9386), rewritten into
//! what both take:
//! - a function with `sampler2D` parameters is copied once per set of samplers its callers pass
//!   (all of them a shader's global samplers), the copy reading those samplers by name;
//! - an unsized array (`const float[] w = float[](...)`, `float w[] = ...`) takes the size its
//!   initializer gives;
//! - an array declared with a `const int`'s size takes the constant's expression.

use std::ops::Range;

use super::located::Lines;

/// The longest shader rewritten (vanilla's, includes expanded, are a few KiB).
const MAX_LEN: usize = 64 * 1024;

/// The bytes the rewrite may scan for calls (the whole text per call it rewrites): thousands of
/// calls leave the text as it was rather than freeze a load.
const SCAN_BUDGET: usize = 64 << 20;

/// `text` with the rewrites, where they apply; anything they can't follow is left as it was.
/// Copies keep their source positions, including retained parameters and specialized bodies.
pub(super) fn fixups(text: Lines) -> Lines {
    if text.text().len() > MAX_LEN {
        return text;
    }
    size_by_int_constants(size_unsized_arrays(specialize_sampler_params(text)))
}

/// The most `const int`s a shader's arrays are sized by.
const MAX_INT_CONSTANTS: usize = 256;

/// `type name[n]` becomes `type name[(expression)]` where `const int n = expression;` is in scope;
/// accesses (`name[n]`) are left as they are. Two constants of one name and different values in
/// one scope, which only exclusive `#if` branches allow, leave the text as it was; the same one
/// twice is a guarded include pasted twice.
fn size_by_int_constants(mut lines: Lines) -> Lines {
    let edits = int_constant_sizes(lines.text());
    lines.edit(edits);
    lines
}

/// The edits of [`size_by_int_constants`]: none where it leaves the text as it was.
fn int_constant_sizes(text: &str) -> Vec<(Range<usize>, String)> {
    struct Constant<'a> {
        name: &'a str,
        expression: &'a str,
        from: usize,
        to: usize,
    }
    let bytes = text.as_bytes();
    let mut constants: Vec<Constant> = Vec::new();
    let mut scopes: Vec<Vec<usize>> = vec![Vec::new()];
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => scopes.push(Vec::new()),
            b'}' => {
                for c in scopes.pop().unwrap_or_default() {
                    constants[c].to = i;
                }
                if scopes.is_empty() {
                    return Vec::new();
                }
            }
            b'c' if text[i..].starts_with("const int ")
                && (i == 0 || !is_ident(char::from(bytes[i - 1]))) =>
            {
                let start = i + "const int ".len();
                let Some(end) = text[start..].find([';', '{', '}']).map(|e| start + e) else {
                    break;
                };
                if bytes[end] == b';'
                    && let Some((name, expression)) = text[start..end].split_once('=')
                {
                    let (name, expression) = (name.trim(), expression.trim());
                    if is_name(name) && !expression.is_empty() {
                        let scope = scopes.last_mut().expect("the global scope stays");
                        let same = scope.iter().find(|&&c| constants[c].name == name);
                        match same {
                            Some(&c) if constants[c].expression == expression => {}
                            Some(_) => return Vec::new(),
                            None if constants.len() == MAX_INT_CONSTANTS => {
                                return Vec::new();
                            }
                            None => {
                                scope.push(constants.len());
                                constants.push(Constant {
                                    name,
                                    expression,
                                    from: end,
                                    to: text.len(),
                                });
                            }
                        }
                    }
                }
                i = end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    if constants.is_empty() {
        return Vec::new();
    }
    let mut edits = Vec::new();
    let mut at = 0;
    while let Some(found) = text[at..].find('[') {
        let open = at + found;
        at = open + 1;
        let Some(close) = text[open..].find(']').map(|c| open + c) else {
            break;
        };
        let size = text[open + 1..close].trim();
        let Some(constant) = constants
            .iter()
            .filter(|c| c.name == size && c.from < open && open < c.to)
            .max_by_key(|c| c.from)
        else {
            continue;
        };
        if !declares_an_array(text, open) {
            continue;
        }
        edits.push((open + 1..close, format!("({})", constant.expression)));
    }
    edits
}

/// Whether the `[` at `open` follows `type name`, so it sizes a declaration.
fn declares_an_array(text: &str, open: usize) -> bool {
    ident_before(text, open)
        .and_then(|(start, _)| ident_before(text, start))
        .is_some_and(|(_, ty)| !matches!(ty, "return" | "else" | "case"))
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// A name a sampler can have (`sampler2D ;` gives none, `t[2]` isn't one).
fn is_name(s: &str) -> bool {
    s.chars().all(is_ident) && s.starts_with(|c: char| !c.is_ascii_digit()) && s != "sampler2D"
}

/// The identifier that ends right before byte `end` (whitespace skipped), and where it starts.
fn ident_before(text: &str, end: usize) -> Option<(usize, &str)> {
    let trimmed = text[..end].trim_end();
    let start = trimmed
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_ident(*c))
        .last()?
        .0;
    let ident = &trimmed[start..];
    (!ident.is_empty() && !ident.starts_with(|c: char| c.is_ascii_digit()))
        .then_some((start, ident))
}

/// For each byte of `text`, the byte of the bracket closing the one opening there (counting
/// that kind of bracket only), or `None`: one pass, so a scan asking for many stays linear.
fn closings(text: &str) -> Vec<Option<usize>> {
    let mut out = vec![None; text.len()];
    let mut open: [Vec<usize>; 3] = Default::default();
    for (i, b) in text.bytes().enumerate() {
        let (kind, opens) = match b {
            b'(' => (0, true),
            b')' => (0, false),
            b'[' => (1, true),
            b']' => (1, false),
            b'{' => (2, true),
            b'}' => (2, false),
            _ => continue,
        };
        if opens {
            open[kind].push(i);
        } else if let Some(at) = open[kind].pop() {
            out[at] = Some(i);
        }
    }
    out
}

/// The comma-separated parts of `list` at its own nesting level.
fn split_top(list: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in list.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&list[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if !list[start..].trim().is_empty() || !parts.is_empty() {
        parts.push(&list[start..]);
    }
    parts
}

/// A function at the file's top level whose parameters include a `sampler2D`.
struct SamplerFn {
    name: String,
    /// The whole function, from the start of its return type to just after its `}` or `;`.
    start: usize,
    end: usize,
    return_type_end: usize,
    name_at: usize,
    open_paren: usize,
    /// The trimmed parameter ranges in the input, in declaration order.
    params: Vec<Range<usize>>,
    close_paren: usize,
    /// The indices of the `sampler2D` parameters and their names.
    samplers: Vec<(usize, String)>,
    /// `None` for a prototype.
    body: Option<String>,
    /// Where the body starts (its `{`, or the `;` of a prototype).
    body_at: usize,
}

/// Find sampler helpers and record the byte ranges of the signature pieces they can keep.
fn sampler_functions(text: &str) -> Vec<SamplerFn> {
    let closing = closings(text);
    let mut out = Vec::new();
    let mut depth = 0i32;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'(' if depth == 0 => {
                if let (Some((name_at, name)), Some(close)) = (ident_before(text, i), closing[i])
                    && let Some((type_at, return_type)) = ident_before(text, name_at)
                {
                    let params: Vec<Range<usize>> = split_top(&text[i + 1..close])
                        .into_iter()
                        .map(|p| {
                            let trimmed = p.trim_start();
                            let at = p.as_ptr() as usize - text.as_ptr() as usize;
                            let start = at + p.len() - trimmed.len();
                            start..start + trimmed.trim_end().len()
                        })
                        .collect();
                    let samplers: Vec<(usize, String)> = params
                        .iter()
                        .map(|range| &text[range.clone()])
                        .enumerate()
                        .filter(|(_, p)| p.split_whitespace().any(|w| w == "sampler2D"))
                        .map(|(n, p)| (n, p.split_whitespace().last().unwrap_or("").to_owned()))
                        .collect();
                    // One without a name is left for naga to refuse.
                    if !samplers.iter().all(|(_, name)| is_name(name)) {
                        i = close + 1;
                        continue;
                    }
                    let after = text[close + 1..].trim_start();
                    let next = close + 1 + (text[close + 1..].len() - after.len());
                    if !samplers.is_empty() && (after.starts_with('{') || after.starts_with(';')) {
                        let (end, body) = if after.starts_with('{') {
                            let Some(body_end) = closing[next] else {
                                break;
                            };
                            (body_end + 1, Some(text[next..=body_end].to_owned()))
                        } else {
                            (next + 1, None)
                        };
                        let body_at = next;
                        out.push(SamplerFn {
                            name: name.to_owned(),
                            start: type_at,
                            end,
                            return_type_end: type_at + return_type.len(),
                            name_at,
                            open_paren: i,
                            params,
                            close_paren: close,
                            samplers,
                            body,
                            body_at,
                        });
                        i = end;
                        continue;
                    }
                }
                // Any other header: no other one sits in its parentheses.
                if let Some(close) = closing[i] {
                    i = close + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The calls to `name` outside `skip` (its own header): each call's span and arguments. The
/// bytes read are added to `scanned`.
fn calls<'a>(
    scanned: &std::cell::Cell<usize>,
    text: &'a str,
    name: &str,
    skip: &[(usize, usize)],
) -> Vec<(usize, usize, Vec<&'a str>)> {
    scanned.set(scanned.get() + text.len());
    let closing = closings(text);
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find(name) {
        let at = from + found;
        from = at + name.len();
        let before = text[..at].chars().next_back();
        let rest = &text[at + name.len()..];
        let open = at + name.len() + (rest.len() - rest.trim_start().len());
        if before.is_some_and(is_ident)
            || !text[open..].starts_with('(')
            || skip.iter().any(|(s, e)| (*s..*e).contains(&at))
        {
            continue;
        }
        let Some(close) = closing[open] else {
            continue;
        };
        scanned.set(scanned.get() + close - open);
        out.push((at, close + 1, split_top(&text[open + 1..close])));
    }
    out
}

/// Replace each whole identifier `from` with `to`, but not a member after a `.` (a
/// parameter `s` leaves the swizzle `c.s` alone). Unchanged bytes keep their positions.
fn replace_ident(lines: &mut Lines, from: &str, to: &str) {
    if from.is_empty() {
        return;
    }
    let text = lines.text();
    let edits = text
        .match_indices(from)
        .filter_map(|(at, _)| {
            let end = at + from.len();
            let before = text[..at].trim_end().chars().next_back();
            let joined = text[..at].chars().next_back().is_some_and(is_ident);
            let after = text[end..].chars().next();
            if joined || before == Some('.') || after.is_some_and(is_ident) {
                return None;
            }
            Some((at..end, to.to_owned()))
        })
        .collect();
    lines.edit(edits);
}

/// A body with its sampler parameters reading `chosen`, all at once: a parameter named like
/// another's sampler (`mix2(sampler2D Sampler0, sampler2D Sampler1)` called with them swapped)
/// still reads its own.
fn read_samplers(body: &mut Lines, params: &[(usize, String)], chosen: &[String]) {
    for (n, (_, param)) in params.iter().enumerate() {
        replace_ident(body, param, &format!("hz_sampler_param_{n}"));
    }
    for (n, sampler) in chosen.iter().enumerate().take(params.len()) {
        replace_ident(body, &format!("hz_sampler_param_{n}"), sampler);
    }
}

/// The byte after `uniform sampler2D <name>;`, where the shader declares that sampler.
fn sampler_declaration_end(text: &str, name: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(found) = text[from..].find("sampler2D") {
        let at = from + found;
        from = at + "sampler2D".len();
        let before = text[..at].trim_end();
        if !before.ends_with("uniform")
            || before[..before.len() - 7]
                .chars()
                .next_back()
                .is_some_and(is_ident)
        {
            continue;
        }
        let rest = &text[from..];
        let ident = rest.trim_start();
        let len = ident.chars().take_while(|c| is_ident(*c)).count();
        if &ident[..len] != name {
            continue;
        }
        let after = ident[len..].trim_start();
        if after.starts_with(';') {
            return Some(text.len() - after.len() + 1);
        }
    }
    None
}

/// The samplers a call passes for `f`'s `sampler2D` parameters, in order.
fn chosen_samplers(f: &SamplerFn, args: &[&str]) -> Vec<String> {
    f.samplers
        .iter()
        .filter_map(|(n, _)| args.get(*n).map(|a| a.trim().to_owned()))
        .collect()
}

/// A call of the specialized name without its sampler arguments. The name points to the
/// original call, and the remaining arguments keep their own source positions.
fn specialized_call(
    source: &Lines,
    at: usize,
    f: &SamplerFn,
    chosen: &[String],
    args: &[&str],
) -> Lines {
    let origin = source.origin_at(at);
    let mut out = Lines::default();
    out.push_replacement(
        &format!("{}_hz_{}(", f.name, chosen.join("_")),
        origin,
        source,
        at,
    );
    let retained = args
        .iter()
        .enumerate()
        .filter(|(n, _)| !f.samplers.iter().any(|(s, _)| s == n));
    for (i, (_, arg)) in retained.enumerate() {
        if i != 0 {
            out.push(", ", origin);
        }
        let arg = arg.trim();
        if !arg.is_empty() {
            let start = arg.as_ptr() as usize - source.text().as_ptr() as usize;
            out.push_lines(&source.slice(start..start + arg.len()));
        }
    }
    out.push(")", origin);
    out
}

/// One pass per function with `sampler2D` parameters, until none is left or one can't be
/// followed (a caller passes something that is no sampler's name). A specialized signature
/// keeps the source positions of the return type, parentheses and retained parameters.
fn specialize_sampler_params(original: Lines) -> Lines {
    let mut text = original.text().to_owned();
    // The text with where each line came from, edited with it.
    let mut lines = original.clone();
    // Bytes scanned for calls so far, against [`SCAN_BUDGET`].
    let scanned = std::cell::Cell::new(0usize);
    let spent = || scanned.get() > SCAN_BUDGET;
    for _ in 0..16 {
        let functions = sampler_functions(&text);
        // From the outside in: no call inside another function still to be copied.
        let inside_another = |f: &SamplerFn, at: usize| {
            functions
                .iter()
                .any(|g| g.name != f.name && g.body.is_some() && (g.start..g.end).contains(&at))
        };
        let spans_of = |f: &SamplerFn| -> Vec<(usize, usize)> {
            functions
                .iter()
                .filter(|g| g.name == f.name)
                .map(|g| (g.start, g.end))
                .collect()
        };
        let Some(f) = functions.iter().find(|f| {
            f.body.is_some()
                && !spent()
                && calls(&scanned, &text, &f.name, &spans_of(f))
                    .iter()
                    .all(|(at, ..)| !inside_another(f, *at))
        }) else {
            if spent() {
                break;
            }
            // None left to copy.
            return lines;
        };
        let spans = spans_of(f);
        let found = calls(&scanned, &text, &f.name, &spans);
        // Each call's samplers must be plain names (a global's, or a later pass's parameter).
        let mut variants: Vec<Vec<String>> = Vec::new();
        let mut call_starts: Vec<usize> = Vec::new();
        let mut followable = true;
        for (at, _, args) in &found {
            let chosen = chosen_samplers(f, args);
            if chosen.len() != f.samplers.len() || !chosen.iter().all(|a| is_name(a)) {
                followable = false;
                break;
            }
            call_starts.push(*at);
            if !variants.contains(&chosen) {
                variants.push(chosen);
            }
            if variants.len() > 64 {
                followable = false;
                break;
            }
        }
        // Overloads (`tap(sampler2D, vec2)` and `tap(sampler2D, vec3)`) each keep their own
        // body; a call picks its samplers by position, so they must sit at the same places.
        let same_places = functions.iter().filter(|g| g.name == f.name).all(|g| {
            g.samplers
                .iter()
                .map(|s| s.0)
                .eq(f.samplers.iter().map(|s| s.0))
        });
        if !followable || !same_places || spent() {
            break;
        }
        // A body may call another overload of itself with its own sampler (vanilla's
        // `sampleNearest` calls the six-argument one): once its sampler parameters read a
        // sampler, those calls name that copy too, and their sets join the variants.
        let rewrite_own_calls = |mut body: Lines| -> Option<(Lines, Vec<Vec<String>>)> {
            let mut found = Vec::new();
            let mut starts: Vec<usize> = calls(&scanned, body.text(), &f.name, &[])
                .iter()
                .map(|c| c.0)
                .collect();
            starts.sort_unstable_by(|a, b| b.cmp(a));
            for start in starts {
                if spent() {
                    return None;
                }
                let (end, chosen, with) = {
                    let (_, end, args) = calls(&scanned, body.text(), &f.name, &[])
                        .into_iter()
                        .find(|c| c.0 == start)?;
                    let chosen = chosen_samplers(f, &args);
                    let with = specialized_call(&body, start, f, &chosen, &args);
                    (end, chosen, with)
                };
                if chosen.len() != f.samplers.len() || !chosen.iter().all(|a| is_name(a)) {
                    return None;
                }
                found.push(chosen);
                body.replace_lines(start..end, with);
            }
            Some((body, found))
        };
        let specialize = |g: &SamplerFn, chosen: &[String]| {
            let mut body = lines.slice(g.body_at..g.end);
            read_samplers(&mut body, &g.samplers, chosen);
            rewrite_own_calls(body)
        };
        let same: Vec<&SamplerFn> = functions.iter().filter(|g| g.name == f.name).collect();
        let mut next = 0;
        while followable && next < variants.len() {
            let chosen = variants[next].clone();
            for g in same.iter().filter(|g| g.body.is_some()) {
                match specialize(g, &chosen) {
                    Some((_, inner)) => {
                        for v in inner {
                            if !variants.contains(&v) {
                                variants.push(v);
                            }
                        }
                    }
                    None => followable = false,
                }
            }
            next += 1;
            followable &= variants.len() <= 64 && !spent();
        }
        if !followable {
            break;
        }
        // One copy per set of samplers, including for prototypes. The signature keeps the
        // spans it copies; the body's fallback line origins stay line for line when possible.
        let copies_of = |g: &SamplerFn| -> Lines {
            let header = lines.origin_at(g.start);
            let copy = |out: &mut Lines, range: Range<usize>| {
                out.push_spanned(&lines.text()[range.clone()], header, &lines, range);
            };
            let write = |out: &mut Lines, code: &str| {
                out.push_replacement(code, header, &lines, g.start);
            };
            let mut copies = Lines::default();
            for chosen in &variants {
                copy(&mut copies, g.start..g.return_type_end);
                write(&mut copies, " ");
                copies.push_replacement(
                    &format!("{}_hz_{}", g.name, chosen.join("_")),
                    header,
                    &lines,
                    g.name_at,
                );
                copy(&mut copies, g.open_paren..g.open_paren + 1);
                let retained = g
                    .params
                    .iter()
                    .enumerate()
                    .filter(|(n, _)| !g.samplers.iter().any(|(s, _)| s == n));
                for (i, (_, range)) in retained.enumerate() {
                    if i != 0 {
                        write(&mut copies, ", ");
                    }
                    copy(&mut copies, range.clone());
                }
                copy(&mut copies, g.close_paren..g.close_paren + 1);
                match &g.body {
                    Some(written) => {
                        let body = specialize(g, chosen).map(|(b, _)| b).unwrap_or_default();
                        write(&mut copies, " ");
                        let from = lines.origins_from(g.body_at);
                        let text = body.text();
                        if text.matches('\n').count() == written.matches('\n').count() {
                            let mut at = 0;
                            for (n, piece) in text.split_inclusive('\n').enumerate() {
                                copies.push_spanned(piece, from[n], &body, at..at + piece.len());
                                at += piece.len();
                            }
                        } else {
                            copies.push_spanned(text, from[0], &body, 0..text.len());
                        }
                        copies.push("\n", header);
                    }
                    None => {
                        copy(&mut copies, g.body_at..g.end);
                        write(&mut copies, "\n");
                    }
                }
            }
            copies
        };
        // Rebuild from the back: the calls, then the function's own spans (each prototype and
        // definition becomes its copies). A call nested in another's arguments starts after it,
        // so it is rewritten first, and the call around it is read again from the text as it is
        // then. A copy reads its samplers by name, so definitions above their samplers'
        // declaration (vanilla's `sample_lightmap.glsl` is included above `uniform sampler2D
        // Sampler2;`) move after it, together and in their order, as long as no call comes first.
        let first_call = call_starts.iter().copied().min().unwrap_or(usize::MAX);
        let declared = variants
            .iter()
            .flatten()
            .filter_map(|s| sampler_declaration_end(&text, s))
            .max();
        let mut edits: Vec<(usize, Option<(usize, Lines)>)> =
            call_starts.into_iter().map(|at| (at, None)).collect();
        let (mut moved, mut move_to) = (Lines::default(), None);
        for g in &same {
            match (
                &g.body,
                declared.filter(|&end| end > g.start && end <= first_call),
            ) {
                (Some(_), Some(end)) => {
                    edits.push((g.start, Some((g.end, Lines::default()))));
                    moved.push_lines(&copies_of(g));
                    move_to = Some(end);
                }
                _ => edits.push((g.start, Some((g.end, copies_of(g))))),
            }
        }
        if let Some(end) = move_to {
            let mut with = Lines::uniform("\n", lines.origin_at(end));
            with.push_lines(&moved);
            edits.push((end, Some((end, with))));
        }
        // Pack text is untrusted: edits that overlap, or off the text, leave it as it was.
        let mut spans: Vec<(usize, usize)> = edits
            .iter()
            .filter_map(|(s, e)| e.as_ref().map(|(end, _)| (*s, *end)))
            .collect();
        spans.sort_unstable();
        if spans.windows(2).any(|w| w[1].0 < w[0].1) {
            break;
        }
        edits.sort_by_key(|(start, _)| std::cmp::Reverse(*start));
        let mut sound = true;
        for (start, edit) in edits {
            let (end, with) = match edit {
                Some(edit) => edit,
                None => {
                    let found = calls(&scanned, lines.text(), &f.name, &[])
                        .into_iter()
                        .find(|c| c.0 == start);
                    let Some((_, end, args)) = found else {
                        continue;
                    };
                    let chosen = chosen_samplers(f, &args);
                    (end, specialized_call(&lines, start, f, &chosen, &args))
                }
            };
            if start > end || text.get(start..end).is_none() || spent() {
                sound = false;
                break;
            }
            text.replace_range(start..end, with.text());
            lines.replace_lines(start..end, with);
        }
        if !sound {
            break;
        }
    }
    // One that couldn't be followed: the shader as written, never half rewritten.
    original
}

/// Each array declared without a size that an initializer sizes: `T[] name = T[](a, b)` and
/// `T name[] = T[](a, b)` (or `T[2](...)`) are written with the size.
fn size_unsized_arrays(mut lines: Lines) -> Lines {
    let mut text = lines.text().to_owned();
    let mut closing = closings(&text);
    // Each array sized reads the text's brackets again: a few hundred are plenty.
    let mut left = 256;
    let mut from = 0;
    while let Some(found) = text[from..].find('=') {
        let eq = from + found;
        from = eq + 1;
        // `==`, `<=`, `>=`, `!=`, `+=` ... are no initializers.
        let prev = text[..eq].chars().next_back();
        let next = text[eq + 1..].chars().next();
        if matches!(
            prev,
            Some('=' | '<' | '>' | '!' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '^')
        ) || next == Some('=')
        {
            continue;
        }
        // The constructor: `T [ n? ] (`.
        let rest = &text[eq + 1..];
        let ty_start = eq + 1 + (rest.len() - rest.trim_start().len());
        let ty_len = text[ty_start..]
            .chars()
            .take_while(|c| is_ident(*c))
            .count();
        if ty_len == 0 {
            continue;
        }
        let after_ty = &text[ty_start + ty_len..];
        let bracket = ty_start + ty_len + (after_ty.len() - after_ty.trim_start().len());
        if !text[bracket..].starts_with('[') {
            continue;
        }
        let Some(bracket_end) = closing[bracket] else {
            continue;
        };
        let after_bracket = &text[bracket_end + 1..];
        let paren = bracket_end + 1 + (after_bracket.len() - after_bracket.trim_start().len());
        if !text[paren..].starts_with('(') {
            continue;
        }
        let Some(paren_end) = closing[paren] else {
            continue;
        };
        // The declarator: `name []` or `T [] name`, between the statement's start and `=`.
        // (Looked for on the line, at most 256 bytes back, so a long line stays linear.)
        let mut back = eq.saturating_sub(256);
        while !text.is_char_boundary(back) {
            back += 1;
        }
        let statement_start = text[back..eq]
            .rfind([';', '{', '}', '\n'])
            .map_or(back, |i| back + i + 1);
        let declarator = &text[statement_start..eq];
        let Some(empty) = declarator.rfind('[') else {
            continue;
        };
        let inside = &declarator[empty + 1..];
        let Some(close) = inside.find(']') else {
            continue;
        };
        if !inside[..close].trim().is_empty() {
            continue;
        }
        let size = split_top(&text[paren + 1..paren_end]).len().to_string();
        // Sized in the constructor too, where it isn't.
        if text[bracket + 1..bracket_end].trim().is_empty() {
            text.replace_range(bracket + 1..bracket_end, &size);
            lines.replace(bracket + 1..bracket_end, &size);
        }
        let at = statement_start + empty + 1;
        text.replace_range(at..at + close, &size);
        lines.replace(at..at + close, &size);
        // The declarator is before `=`, so `=` moved by what its size changed.
        from = eq + 1 + size.len() - close;
        closing = closings(&text);
        left -= 1;
        if left == 0 {
            break;
        }
    }
    lines
}

#[cfg(test)]
mod tests;
