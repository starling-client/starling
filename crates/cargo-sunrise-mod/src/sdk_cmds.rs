//! Commands that read SDK data: `check` of a mod folder, `check --build <name>`, `hooks <text>`,
//! `sdk diff <a> <b>`, and the report `pin` prints before it repins. Each reads the pinned build's
//! `surface.json` from `.sunrise/sdk/`, or another build's
//! small files through [`crate::sdk_fetch::fetch_build`]; none links the generator's catalog.
use crate::options::Options;
use crate::sdk_fetch::{PinnedSdk, ensure_sdk, fetch_build, fetch_index_only, fetch_pinned};
use crate::sdk_notes::{self, Shipped};
use crate::uses_table::{self, Scanned, surface_of};
use mod_surface::child::ChildEntry;
use mod_surface::uses::{Change, Fit, USES_FILE, UsesTable};
use mod_surface::{Surface, leaf};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use sunrise_mod_package::{
    Manifest, Package,
    manifest::MAX_USES,
    sdk::{self, Index},
};
use sunrise_native_build::check::{self, Builds, Judged, Unlisted, Verdict};
use sunrise_native_build::game_uses;

/// One searchable record: its id, its doc text and its names in other mod APIs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Record<'s> {
    pub id: &'s str,
    pub doc: &'s str,
    pub counterparts: Vec<String>,
}

/// Every record of `surface` a search reads: hooks only, or every kind with its members.
fn records(surface: &Surface, hooks_only: bool) -> Vec<Record<'_>> {
    let mut out: Vec<Record<'_>> = surface
        .hooks
        .iter()
        .map(|hook| {
            let names = &hook.counterparts;
            let counterparts = names
                .neoforge
                .iter()
                .map(|name| format!("NeoForge: {name}"))
                .chain(names.fabric.iter().map(|name| format!("Fabric: {name}")))
                .chain(names.paper.iter().map(|name| format!("Paper: {name}")))
                .collect();
            Record {
                id: &hook.id,
                doc: &hook.doc,
                counterparts,
            }
        })
        .collect();
    if hooks_only {
        return out;
    }
    for item in &surface.traits {
        out.push(Record {
            id: &item.id,
            doc: &item.doc,
            counterparts: vec![],
        });
        for method in &item.methods {
            out.push(Record {
                id: &method.id,
                doc: &method.doc,
                counterparts: vec![],
            });
        }
    }
    for function in surface.handles.iter().chain(&surface.functions) {
        out.push(Record {
            id: &function.id,
            doc: &function.doc,
            counterparts: vec![],
        });
    }
    for item in &surface.macros {
        out.push(Record {
            id: &item.id,
            doc: &item.doc,
            counterparts: vec![],
        });
    }
    for item in &surface.types {
        out.push(Record {
            id: &item.id,
            doc: &item.doc,
            counterparts: vec![],
        });
    }
    for item in &surface.parts {
        out.push(Record {
            id: &item.id,
            doc: &item.doc,
            counterparts: vec![],
        });
    }
    for item in &surface.capabilities {
        out.push(Record {
            id: &item.id,
            doc: &item.doc,
            counterparts: vec![],
        });
    }
    out
}

/// The records every word of `text` occurs in (id, doc or counterparts, any case): those whose
/// id holds every word first, then those a counterpart completes, then the rest, each by id.
pub(crate) fn search<'s>(surface: &'s Surface, text: &str, hooks_only: bool) -> Vec<Record<'s>> {
    let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return vec![];
    }
    let mut found: Vec<(u8, Record<'s>)> = records(surface, hooks_only)
        .into_iter()
        .filter_map(|record| {
            let id = record.id.to_lowercase();
            let doc = record.doc.to_lowercase();
            let counterparts = record.counterparts.join(" ").to_lowercase();
            let rank = if words.iter().all(|w| id.contains(w)) {
                0
            } else if words
                .iter()
                .all(|w| id.contains(w) || counterparts.contains(w))
            {
                1
            } else if words
                .iter()
                .all(|w| id.contains(w) || counterparts.contains(w) || doc.contains(w))
            {
                2
            } else {
                return None;
            };
            Some((rank, record))
        })
        .collect();
    found.sort_by(|a, b| (a.0, a.1.id).cmp(&(b.0, b.1.id)));
    found.into_iter().map(|(_, record)| record).collect()
}

/// The first line of a doc text.
fn first_line(doc: &str) -> &str {
    doc.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
}

/// `hooks <text> [--all]`: the search words and whether `--all` was given.
fn hooks_args(args: &[String]) -> Result<(String, bool), String> {
    let mut options = Options::new("hooks", args, &[]);
    let all = options.switch("--all");
    let text = options.positional().join(" ");
    options.finish()?;
    if text.trim().is_empty() {
        return Err("cargo sunrise-mod hooks <text> [--all]".into());
    }
    Ok((text, all))
}

/// `hooks <text> [--all]`: the pinned build's hooks (with `--all`, every record) whose id, doc
/// or counterparts hold every word, offline.
pub(crate) fn hooks(args: &[String]) -> Result<(), String> {
    let (text, all) = hooks_args(args)?;
    let sdk = ensure_sdk(Path::new("."))?;
    let surface = surface_of(&sdk)?;
    let found = search(&surface, &text, !all);
    if found.is_empty() {
        let what = if all { "item" } else { "hook" };
        println!("No {what} of {} matches `{text}`.", sdk.name);
        return Ok(());
    }
    for record in &found {
        let names = if record.counterparts.is_empty() {
            String::new()
        } else {
            format!("  ({})", record.counterparts.join(", "))
        };
        println!("{}{names}", record.id);
        let line = first_line(record.doc);
        if !line.is_empty() {
            println!("    {line}");
        }
    }
    Ok(())
}

/// `sdk diff <a> <b>`: what changed in the surface from build `a` to build `b` (release names
/// or shas, as `--sunrise` takes them). Only each build's small files are read, so a pruned build
/// still answers.
pub(crate) fn sdk(args: &[String]) -> Result<(), String> {
    let usage = "cargo sunrise-mod sdk diff <build> <build>";
    let [command, a, b] = args else {
        return Err(usage.into());
    };
    if command != "diff" {
        return Err(usage.into());
    }
    let project = Path::new(".");
    let old = fetch_build(project, a)?;
    let new = fetch_build(project, b)?;
    let chain = sdk_notes::chain(&new, Some(&old.build), |build| {
        fetch_index_only(project, build)
    })?;
    println!("{} to {}", old.name, new.name);
    print!(
        "{}",
        diff_text(&surface_of(&old)?, &surface_of(&new)?, &chain.notes)
    );
    if let Some(line) = chain.stopped_line() {
        println!("{line}");
    }
    Ok(())
}

/// The diff from `old` to `new`, with the change `notes` of the builds between them under the
/// changes they name, each tagged with the build that carried it.
fn diff_text(old: &Surface, new: &Surface, notes: &[Shipped]) -> String {
    let mut diff = mod_surface::diff(old, new);
    for shipped in notes {
        let text = mod_surface::diff::Note {
            file: shipped.build.clone(),
            why: shipped.note.why.clone(),
            migrate: shipped.note.migrate.clone(),
        };
        // An id the note names may be unchanged here when `old` is not the build the note's
        // build was compared with; the note then explains a change this diff does not show.
        diff.attach(&text, &shipped.note.ids);
    }
    diff.to_text()
}

/// What a fit check judges: the package's or the folder's table, or why the question does not apply.
pub(crate) enum Subject {
    Table {
        name: String,
        table: UsesTable,
        /// The mod's manifest: the game code it declares is judged with the list.
        manifest: Box<Manifest>,
    },
    /// The answer to print with exit status 2.
    NotApplicable { message: String },
}

/// What the package at `path` gives `fits` to judge.
pub(crate) fn package_subject(path: &Path) -> Result<Subject, String> {
    package_uses(path, Package::read(path)?)
}

fn package_uses(path: &Path, package: Package) -> Result<Subject, String> {
    let name = &package.manifest.name;
    if let Some(table) = game_uses::unjudged(&package.manifest) {
        return Ok(Subject::NotApplicable {
            message: exact_text(name, &format!("it has a {table} table")),
        });
    }
    let Some(native) = &package.manifest.native else {
        return Ok(Subject::NotApplicable {
            message: content_text(name),
        });
    };
    let pin = &native.sunrise;
    let Some(bytes) = Package::entry(path, USES_FILE, MAX_USES)? else {
        return Ok(Subject::NotApplicable {
            message: format!(
                "{name} pins build {pin} and its package carries no list of what it uses, so it can't be shown to fit another build. Pack it with `cargo sunrise-mod build`, which writes the list when the mod's code can be read, or repin it with `cargo sunrise-mod pin <build>` first."
            ),
        });
    };
    let table = UsesTable::read(&bytes).map_err(|e| format!("{USES_FILE}: {e}"))?;
    if *pin != table.made_for {
        return Ok(Subject::NotApplicable {
            message: format!(
                "{name} pins build {pin} and its list of what it uses was made for {}, so the list says nothing about the pinned build. Pack it again with `cargo sunrise-mod build`.",
                table.made_for
            ),
        });
    }
    Ok(Subject::Table {
        name: name.clone(),
        table,
        manifest: Box::new(package.manifest),
    })
}

/// The table of the mod folder at `root`, scanned against its pinned build.
fn folder_subject(entry: ChildEntry, root: &Path) -> Result<Subject, String> {
    let parsed = crate::read_manifest(root)?;
    let name = &parsed.name;
    let Some(native) = &parsed.native else {
        return Ok(Subject::NotApplicable {
            message: content_text(name),
        });
    };
    if let Some(table) = game_uses::unjudged(&parsed) {
        return Ok(Subject::NotApplicable {
            message: exact_text(name, &format!("it has a {table} table")),
        });
    }
    let sdk = fetch_pinned(root, &native.sunrise)?;
    match uses_table::scan(entry, root, &parsed, &surface_of(&sdk)?)? {
        Scanned::Uses(uses) => Ok(Subject::Table {
            name: name.clone(),
            table: uses_table::table(&uses, &parsed, &sdk)?,
            manifest: Box::new(parsed),
        }),
        Scanned::Exact(why) => Ok(Subject::NotApplicable {
            message: exact_text(name, &why),
        }),
        Scanned::Unread(why) => Err(why),
    }
}

fn content_text(name: &str) -> String {
    format!("{name} is a content mod, with no code to check against a build.")
}

fn exact_text(name: &str, why: &str) -> String {
    format!("{name} is made for exactly one build ({why}), so the fit check doesn't apply.")
}

/// `check [<file.smod>] --build <name> [--build <name>]…`: reports fit for each build. Exits 0
/// when every build fits, 1 when one does not, and 2 when it cannot tell or does not apply.
pub(crate) fn check_fit(entry: ChildEntry, package: Option<&Path>, builds: &[&str]) -> ExitCode {
    match fit_lines(entry, package, builds) {
        Ok(Answer::Fits) => ExitCode::SUCCESS,
        Ok(Answer::DoesNotFit) => ExitCode::from(1),
        Ok(Answer::Unknown) => ExitCode::from(2),
        // An answer, not a failure: printed plainly.
        Ok(Answer::NotApplicable(text)) => {
            println!("{text}");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

/// The result of a fit check, once its lines are printed.
enum Answer {
    Fits,
    DoesNotFit,
    /// No build misfits, and for one the list cannot say; its line is printed.
    Unknown,
    /// The question does not apply: why, not yet printed.
    NotApplicable(String),
}

fn fit_lines(entry: ChildEntry, package: Option<&Path>, builds: &[&str]) -> Result<Answer, String> {
    let project = Path::new(".");
    let subject = match package {
        Some(path) => package_subject(path)?,
        None => folder_subject(entry, project)?,
    };
    let (name, table, manifest) = match subject {
        Subject::Table {
            name,
            table,
            manifest,
        } => (name, table, manifest),
        Subject::NotApplicable { message, .. } => return Ok(Answer::NotApplicable(message)),
    };
    let made_for = table.made_for.clone();
    // The surface of the build the list was made for, fetched at most once whatever the builds:
    // asked for only when a target leaves nothing else to judge.
    let mut made_for_surface: Option<Result<Vec<u8>, String>> = None;
    println!("{name}:");
    let (mut all, mut known) = (true, true);
    // Each build gets its own verdict: one that cannot be fetched or compared is not known, and
    // the rest are still checked.
    for build in builds {
        let target = match fetch_build(project, build) {
            Ok(target) => target,
            Err(why) => {
                known = false;
                println!("{build}  not known: {why}");
                continue;
            }
        };
        // The identity comes first: a mod made for this build is answered by the build's id,
        // whatever its own surface says.
        if target.build == table.made_for {
            println!("{}  made for", target.name);
            continue;
        }
        let target_surface = match surface_of(&target) {
            Ok(surface) => surface,
            Err(why) => {
                known = false;
                println!("{}  not known: {why}", target.name);
                continue;
            }
        };
        let target_surface = Rc::new(target_surface);
        // Each build judges its own copy, which the made-for surface may complete: what one
        // build's verdict needed never changes another's.
        let mut target_table = table.clone();
        let judged = mixins_rev(&target).and_then(|mixins_rev| {
            check::judge(
                &manifest,
                Builds {
                    made_for: &made_for,
                    build: &target.build,
                    mixins_rev: &mixins_rev,
                },
                || Ok(&mut target_table),
                &mut || Ok(target_surface.clone()),
                &mut || build_data(&manifest, &target),
                Some(&mut || {
                    made_for_surface
                        .get_or_insert_with(|| {
                            fetch_build(project, &made_for).and_then(|made| {
                                std::fs::read(&made.surface)
                                    .map_err(|e| format!("{}: {e}", made.surface.display()))
                            })
                        })
                        .clone()
                }),
            )
        });
        let Judged {
            verdict,
            deferred,
            uncompared,
            unjudged,
            unpinned,
        } = match judged {
            Ok(judged) => judged,
            Err(why) => {
                known = false;
                println!("{}  not known: {why}", target.name);
                continue;
            }
        };
        match verdict {
            Verdict::MadeFor => println!("{}  made for", target.name),
            Verdict::Fits { .. } => println!("{}  fits", target.name),
            Verdict::Exact { .. } => {
                known = false;
                println!(
                    "{}  not known: nothing the list can judge changed, but the build's data cannot show {} unchanged (it holds no body for a function the mod calls, or not every part of a loop).",
                    target.name,
                    unjudged
                        .iter()
                        .map(|id| format!("`{}`", id.rsplit(':').next().unwrap_or(id)))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Verdict::NoList {
                why: Unlisted::GameCode,
                ..
            } => {
                known = false;
                println!(
                    "{}  not known: nothing the list can judge changed, but it holds no record of game code the mod declares. Pack the mod again with `cargo sunrise-mod build`, which records it.",
                    target.name
                );
            }
            // The list is the pin's, so only its hook versions can be missing.
            Verdict::NoList { hooks, .. } => {
                known = false;
                println!(
                    "{}  not known: nothing the list can judge changed, but it holds no version of {}{}. Pack the mod again with `cargo sunrise-mod build`, which records them.",
                    target.name,
                    hooks
                        .iter()
                        .map(|id| format!("`{}`", leaf(id)))
                        .collect::<Vec<_>>()
                        .join(", "),
                    unpinned.map_or(String::new(), |why| format!(
                        ", and the build it was made for could not be read for them ({why})"
                    )),
                );
            }
            Verdict::DoesNotFit { misfits, .. } => {
                all = false;
                let count = misfits
                    .iter()
                    .map(|m| m.id.as_str())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                let items = if count == 1 { "item" } else { "items" };
                println!("{}  does not fit: {count} {items} changed", target.name);
                for misfit in misfits {
                    println!("    {misfit}");
                }
                if let Some(why) = uncompared {
                    println!("    the game code it declares could not be compared: {why}");
                }
            }
        }
        if let Some(note) = deferred {
            println!("    {note}");
        }
    }
    Ok(match (all, known) {
        (false, _) => Answer::DoesNotFit,
        (true, false) => Answer::Unknown,
        (true, true) => Answer::Fits,
    })
}

/// The horizon-mixins revision `target`'s tool builds with, from its verified `sdk.json`.
fn mixins_rev(target: &PinnedSdk) -> Result<String, String> {
    let path = target.dir.join(sdk::INDEX);
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let index =
        Index::read(&bytes, &target.build).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(index.mixins.rev)
}

/// Where the game code `manifest` declares is in `target`, by its build data. The build data is
/// fetched from `target`'s signed SDK into a private folder, never read from the project's
/// cache, which the checkout can edit.
fn build_data(
    manifest: &Manifest,
    target: &PinnedSdk,
) -> Result<Vec<(String, game_uses::Now)>, String> {
    let staging = tempfile::tempdir().map_err(|e| format!("build data: {e}"))?;
    crate::sdk_fetch::release_data(
        &crate::sdk_fetch::Base::from_env(),
        &crate::sdk_fetch::keys(),
        &target.build,
        staging.path(),
        true,
    )?;
    let data = staging
        .path()
        .join(&target.build)
        .join(crate::sdk_fetch::BUILD_DATA);
    game_uses::entries(manifest, &data, &target.build)
}

/// What the mod nearest to a removed item may use now, in `new`: the hook with one of the same
/// counterparts first, then the first record whose id holds every word of the item's name.
pub(crate) fn closest<'s>(old: &Surface, new: &'s Surface, id: &str) -> Option<Record<'s>> {
    let old_names: Vec<String> = records(old, true)
        .into_iter()
        .find(|record| record.id == id)
        .map(|record| record.counterparts)
        .unwrap_or_default();
    if !old_names.is_empty()
        && let Some(record) = records(new, true)
            .into_iter()
            .find(|record| record.counterparts.iter().any(|n| old_names.contains(n)))
    {
        return Some(record);
    }
    let words = leaf(id)
        .rsplit("::")
        .next()
        .unwrap_or(id)
        .split(['_', '.'])
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    search(new, &words, false)
        .into_iter()
        .find(|record| record.id != id)
}

/// What moving the pin from the build the mod is pinned to to `to` (the current Nightly when
/// `None`) breaks in this mod, item by item with the lines that use it; printed before `pin`
/// repins. A mod made for exactly one build is only named.
pub(crate) fn pin_report(entry: ChildEntry, root: &Path, to: &str) -> Result<(), String> {
    let parsed = crate::read_manifest(root)?;
    let name = &parsed.name;
    let exact_line = || {
        println!("{name} is made for exactly one build; its declarations are checked as it moves.")
    };
    let Some(native) = &parsed.native else {
        return Ok(());
    };
    // Known from the manifest: such a mod has no table to compare, so nothing is fetched for it.
    if game_uses::unjudged(&parsed).is_some() {
        exact_line();
        return Ok(());
    }
    let old = fetch_pinned(root, &native.sunrise)?;
    // Moving off a build whose surface schema this tool does not read is how such a mod gets
    // back to a build it does: there is nothing to compare, and `check` judges the new build.
    let old_surface = match uses_table::read_surface(&old)? {
        Ok(surface) => surface,
        Err(e @ mod_surface::ReadError::Schema(_)) => {
            println!(
                "{}: {e}, so what the move changes can't be compared.",
                old.name
            );
            return Ok(());
        }
        Err(e) => return Err(format!("{}: {e}", old.surface.display())),
    };
    let uses = match uses_table::scan(entry, root, &parsed, &old_surface)? {
        Scanned::Uses(uses) => uses,
        Scanned::Exact(_) => {
            exact_line();
            return Ok(());
        }
        Scanned::Unread(why) => return Err(why),
    };
    let new = fetch_build(root, to)?;
    let new_surface = surface_of(&new)?;
    let chain = sdk_notes::chain(&new, Some(&old.build), |build| {
        fetch_index_only(root, build)
    })?;
    let notes = migrate_notes(&chain.notes);
    print!(
        "{}",
        report(name, &old, &new, &uses, &old_surface, &new_surface, &notes)
    );
    if let Some(line) = chain.stopped_line() {
        println!("{line}");
    }
    // The game code the mod declares, by the new build's build data.
    let made = uses_table::table(&uses, &parsed, &old)?;
    // A misfit already known stays when the build data cannot be read: why it cannot comes second.
    let (compared, uncompared) =
        game_uses::compare_declared(&parsed, &made, &mixins_rev(&new)?, || {
            build_data(&parsed, &new)
        });
    for misfit in &compared.misfits {
        println!("{misfit}");
    }
    let mut settled = compared.misfits.is_empty();
    if let Some(why) = uncompared {
        settled = false;
        println!("The game code it declares could not be compared: {why}");
    }
    if compared.unrecorded {
        settled = false;
        println!(
            "The pinned build's data holds no fingerprint for some game code it declares, so nothing can be said of it."
        );
    }
    if !compared.unjudged.is_empty() {
        settled = false;
        println!(
            "The new build's data holds no fingerprint for {}, so nothing can be said of it.",
            compared
                .unjudged
                .iter()
                .map(|id| format!("`{}`", id.rsplit(':').next().unwrap_or(id)))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if settled && game_uses::declared(&parsed) {
        println!("None of the game code it declares changed.");
    }
    Ok(())
}

/// Each changed id's `migrate` line from `notes` with the build that carried it; the newest
/// build's wins, and a note without a line adds nothing.
fn migrate_notes(notes: &[Shipped]) -> BTreeMap<String, (String, String)> {
    let mut out = BTreeMap::new();
    for shipped in notes {
        let Some(migrate) = &shipped.note.migrate else {
            continue;
        };
        for id in &shipped.note.ids {
            out.entry(id.clone())
                .or_insert_with(|| (shipped.build.clone(), migrate.clone()));
        }
    }
    out
}

/// The text of [`pin_report`].
fn report(
    name: &str,
    old: &PinnedSdk,
    new: &PinnedSdk,
    uses: &mod_surface::uses::Uses,
    old_surface: &Surface,
    new_surface: &Surface,
    notes: &BTreeMap<String, (String, String)>,
) -> String {
    let total = uses.items.len();
    let misfits = match uses.fits(new_surface) {
        // A fresh scan names every hook's version.
        Fit::Fits | Fit::Unversioned(_) => vec![],
        Fit::DoesNotFit(misfits) => misfits,
    };
    let mut changed: Vec<&str> = misfits.iter().map(|m| m.id.as_str()).collect();
    changed.dedup();
    let mut text = if changed.is_empty() {
        format!(
            "{name}, from {} to {}: none of the {total} items it uses changed.\n",
            old.name, new.name
        )
    } else {
        format!(
            "{name}, from {} to {}: {} of the {total} items it uses changed.\n\n",
            old.name,
            new.name,
            changed.len()
        )
    };
    for misfit in &misfits {
        let id = &misfit.id;
        text.push_str(&format!("  {misfit}\n"));
        if let Some(used) = uses.items.iter().find(|used| &used.entry.id == id) {
            text.push_str(&format!("      {}\n", uses_table::place(&used.origin)));
        }
        if misfit.change == Change::Removed
            && let Some(record) = closest(old_surface, new_surface, id)
        {
            let names = if record.counterparts.is_empty() {
                String::new()
            } else {
                format!(" ({})", record.counterparts.join(", "))
            };
            text.push_str(&format!("      closest now: {}{names}\n", record.id));
        }
        if let Some((build, migrate)) = notes.get(id) {
            text.push_str(&format!(
                "      note from the change ({build}): \"{migrate}\"\n"
            ));
        }
    }
    text
}

/// Checks a mod folder and reports its API uses. Release checks enforce the page and tag
/// requirements, and compile a native mod instead of only type-checking it.
pub(crate) fn check_mod(
    entry: ChildEntry,
    root: &Path,
    show_uses: bool,
    with: &[&Path],
    release: Option<&str>,
) -> Result<(), String> {
    let package = Package::read(root)?;
    let files = package.directory_files(root)?;
    let prepared_resources = std::env::var_os("SUNRISE_RESOURCES")
        .or_else(|| std::env::var_os("SUNRISE_TEST_RESOURCES"))
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir());
    let (pipelines, refused) =
        crate::check::pipeline_shaders(&files, prepared_resources.as_deref());
    if !refused.is_empty() {
        return Err(format!(
            "render pipeline shaders refused:\n{}",
            refused.join("\n")
        ));
    }
    if pipelines > 0 {
        let source = if prepared_resources.is_some() {
            "prepared resources"
        } else {
            "Sunrise interface stubs (set SUNRISE_RESOURCES for prepared 26.3 includes)"
        };
        println!(
            "{pipelines} GLSL render pipeline program(s) translate and validate as the game does ({source})."
        );
    }
    crate::check::references(root, &package.manifest)?;
    crate::page::check(root, &package, release)?;
    let Some(native) = &package.manifest.native else {
        if !with.is_empty() {
            return Err("check --with supplies libraries to a native mod".into());
        }
        println!("{}: the package rules hold.", package.manifest.name);
        return Ok(());
    };
    // A release builds from the tagged source with its committed lockfile, as the registry does.
    if release.is_some()
        && !std::fs::symlink_metadata(root.join("Cargo.lock")).is_ok_and(|m| m.is_file())
    {
        return Err(
            "a native mod releases with its Cargo.lock committed beside sunrise-mod.toml".into(),
        );
    }
    // The mod's `sunrise-mod-api` is a path into `.sunrise/sdk/`: fetch it before Cargo looks.
    let sdk = fetch_pinned(root, &native.sunrise)?;
    let surface = surface_of(&sdk)?;
    let built = if release.is_some() {
        let rust = crate::dev::native_sdk(&crate::dev::cache_dir()?, &native.sunrise)?.rust;
        crate::dev::check_lock(&rust, root, &native.sunrise)?;
        crate::dev::build(
            root,
            &package,
            with,
            false,
            sunrise_native_build::contract::TargetName::Client,
            true,
        )
        .map(|_| ())
    } else {
        crate::dev::check(root, &package, with)
    };
    if let Err(error) = built {
        // The compiler's error stands; the notes of the changes that may explain it follow it.
        let Ok(Ok(sources)) = uses_table::read(root, &package.manifest) else {
            return Err(error);
        };
        // The notes since the build the mod last compiled against; with none recorded, only the
        // pinned build's own, and the line after them says how to read older ones.
        let last = sdk_notes::last_built(root);
        let walked = sdk_notes::chain(&sdk, last.as_deref(), |build| match &last {
            Some(_) => fetch_index_only(root, build),
            None => Err(format!(
                "no build this mod compiled against is recorded; `cargo sunrise-mod sdk diff <build> {}` lists the notes since one",
                sdk.build
            )),
        });
        let Ok(chain) = walked else {
            return Err(error);
        };
        let explained = sdk_notes::explaining(&chain.notes, &surface, &sources);
        return Err(sdk_notes::with_notes(error, &explained, &chain));
    }
    sdk_notes::record_built(root, &sdk.build)?;
    match uses_table::scan(entry, root, &package.manifest, &surface)? {
        Scanned::Uses(uses) => {
            print!(
                "{}",
                uses_table::summary(&package.manifest.name, &uses, &sdk)
            );
            if show_uses {
                print!("{}", uses_table::list(&uses));
            }
        }
        Scanned::Exact(_) => println!(
            "{} is made for exactly one build ({}); it has no uses table.",
            package.manifest.name, sdk.name
        ),
        Scanned::Unread(why) => return Err(why),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_surface::{Counterparts, Hook, HookContract, HookEngine, HookKind, Side};
    use sunrise_mod_package::manifest::MANIFEST;
    use sunrise_mod_package::sdk::ChangeNote;

    fn hook(id: &str, doc: &str, neoforge: &[&str]) -> Hook {
        Hook::new(
            id,
            doc,
            HookEngine::default(),
            Counterparts {
                neoforge: neoforge.iter().map(|n| n.to_string()).collect(),
                ..Default::default()
            },
            HookContract {
                version: 1,
                side: Side::Server,
                contract: HookKind::Listen,
                scope: None,
                cancellable: None,
                context: None,
                host: None,
                owner: None,
                answered_by_mod: false,
                arguments: vec![],
                output: None,
                refs: vec![],
            },
        )
    }

    fn surface(hooks: Vec<Hook>) -> Surface {
        Surface::new(
            hooks,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
        )
    }

    #[test]
    fn a_search_ranks_id_then_counterpart_then_doc_matches() {
        let surface = surface(vec![
            hook(
                "hook:living_damage_post",
                "After a hit lands.",
                &["LivingDamageEvent.Post"],
            ),
            hook("hook:player_hurt", "When damage reaches a player.", &[]),
            hook(
                "hook:block_broken",
                "A block breaks.",
                &["BlockEvent.BreakEvent"],
            ),
        ]);
        let ids = |text: &str| -> Vec<&str> {
            search(&surface, text, true)
                .into_iter()
                .map(|r| r.id)
                .collect()
        };
        assert_eq!(
            ids("damage"),
            ["hook:living_damage_post", "hook:player_hurt"]
        );
        assert_eq!(ids("breakevent"), ["hook:block_broken"]);
        assert_eq!(ids("DAMAGE post"), ["hook:living_damage_post"]);
        assert!(ids("teleport").is_empty());
    }

    const BUILD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OTHER_BUILD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn packed_subject(kind: &str, sdk: Option<&str>, made_for: &str) -> Subject {
        let manifest = format!(
            "id='greeter'\nname='Greeter'\nversion='0.1.0'\napi='0.2'\ncapabilities=['registries']\n{kind}"
        );
        let mut cargo = "[package]\nname='greeter'\nversion='0.1.0'\n".to_owned();
        if let Some(sdk) = sdk {
            cargo.push_str(&format!("[package.metadata.sunrise]\nsdk='{sdk}'\n"));
        }
        let uses = UsesTable {
            schema: mod_surface::uses::USES_SCHEMA,
            made_for: made_for.into(),
            entries: vec![],
        }
        .to_json();
        let files = [
            ("Cargo.toml", cargo.as_bytes()),
            ("src/lib.rs", b"pub fn greet() {}".as_slice()),
            (USES_FILE, uses.as_bytes()),
        ];
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("greeter.smod");
        std::fs::write(&path, Package::write_files(&manifest, &files).unwrap()).unwrap();
        package_subject(&path).unwrap()
    }

    #[test]
    fn packed_native_uses_follow_the_manifest_pin_not_cargo_metadata() {
        let kind = format!("[native]\ngame='26.3'\nsunrise='{BUILD}'\n");
        assert!(matches!(
            packed_subject(&kind, Some(OTHER_BUILD), BUILD),
            Subject::Table { table, .. } if table.made_for == BUILD
        ));
        assert!(matches!(
            packed_subject(&kind, Some(OTHER_BUILD), OTHER_BUILD),
            Subject::NotApplicable { message, .. } if message.contains("the list says nothing about the pinned build")
        ));
    }

    fn note(ids: &[&str], why: &str, migrate: Option<&str>) -> ChangeNote {
        ChangeNote {
            ids: ids.iter().map(|id| id.to_string()).collect(),
            why: why.into(),
            migrate: migrate.map(str::to_owned),
        }
    }

    fn shipped(build: &str, note: ChangeNote) -> Shipped {
        Shipped {
            build: build.into(),
            note,
        }
    }

    #[test]
    fn a_content_mods_pin_report_does_not_access_the_sdk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(MANIFEST),
            "id = \"greeter\"\nname = \"Greeter\"\nversion = \"0.1.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
        )
        .unwrap();
        assert_eq!(
            pin_report(mod_surface::child::child_entry(), dir.path(), "nightly"),
            Ok(())
        );
        assert!(!dir.path().join(".sunrise").exists());
    }

    #[test]
    fn the_pin_report_takes_each_ids_migrate_line() {
        let notes = migrate_notes(&[
            shipped(
                "newer",
                note(
                    &["method:ServerHost::give_item"],
                    "Parts",
                    Some("items go through Parts under a transaction"),
                ),
            ),
            shipped("newer", note(&["hook:x"], "only why", None)),
            shipped(
                "older",
                note(
                    &["method:ServerHost::give_item"],
                    "first",
                    Some("older line"),
                ),
            ),
        ]);
        assert_eq!(
            notes.get("method:ServerHost::give_item"),
            Some(&(
                "newer".to_owned(),
                "items go through Parts under a transaction".to_owned()
            ))
        );
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn update_report_shows_migration_for_a_removed_world_lifecycle_member() {
        use mod_surface::uses::{Entry, Origin, Used, Uses};
        // Keep the formatter fixture independent of the real lifecycle note's IDs and wording.
        let id = "type:FixturePhase.Gone";
        let uses = Uses {
            items: vec![Used {
                entry: Entry::plain(id.into(), "old-version".into()),
                origin: Origin::Source {
                    file: "src/lib.rs".into(),
                    line: 19,
                },
                note: None,
            }],
        };
        let lifecycle: ChangeNote = toml::from_str(include_str!(
            "../../sunrise-mod-vocab/changes/debt-re-wl-re-wl.toml"
        ))
        .unwrap();
        for changed in [
            "type:WorldPhase.WorldCloseFailed",
            "hook:world_lifecycle",
            "hook:client_level_load",
            "hook:client_level_unload",
        ] {
            assert!(
                lifecycle.ids.iter().any(|id| id == changed),
                "missing ID {changed}"
            );
        }
        assert!(
            lifecycle
                .migrate
                .is_some_and(|text| !text.trim().is_empty())
        );
        let notes = migrate_notes(&[shipped(
            "42c5f70",
            note(
                &[id],
                "The fixture phase was removed.",
                Some("switch to the replacement fixture phase"),
            ),
        )]);
        let pinned = |name: &str, build: &str| PinnedSdk {
            name: name.into(),
            build: build.into(),
            dir: Default::default(),
            surface: Default::default(),
        };
        let text = report(
            "greeter",
            &pinned("old", BUILD),
            &pinned("new", OTHER_BUILD),
            &uses,
            &surface(vec![]),
            &surface(vec![]),
            &notes,
        );
        assert!(
            text.lines()
                .any(|line| line.trim() == "the variant `FixturePhase::Gone` (removed)"),
            "{text}"
        );
        assert!(
            text.lines().any(|line| line.trim()
                == "note from the change (42c5f70): \"switch to the replacement fixture phase\""),
            "{text}"
        );
    }

    #[test]
    fn sdk_diff_prints_the_newer_builds_notes_under_the_changes_they_name() {
        let old = surface(vec![hook("hook:gone", "", &[])]);
        let new = surface(vec![]);
        let text = diff_text(
            &old,
            &new,
            &[shipped(
                "42c5f70",
                note(
                    &["hook:gone"],
                    "It fired twice.",
                    Some("Listen to hook:once."),
                ),
            )],
        );
        assert!(text.contains("0 breaking without a note"), "{text}");
        assert!(
            text.contains("    why (42c5f70): It fired twice.\n"),
            "{text}"
        );
        assert!(text.contains("Listen to hook:once."), "{text}");
    }

    #[test]
    fn the_closest_record_shares_a_counterpart_first() {
        let old = surface(vec![hook(
            "hook:block_broken",
            "",
            &["BlockEvent.BreakEvent"],
        )]);
        let new = surface(vec![
            hook("hook:block_broken_v", "", &[]),
            hook("hook:block_break", "", &["BlockEvent.BreakEvent"]),
        ]);
        assert_eq!(
            closest(&old, &new, "hook:block_broken").map(|r| r.id),
            Some("hook:block_break")
        );
    }

    /// A build folder holding only the `sdk.json` of `commit`, which names `previous` and carries `notes`.
    fn chained(
        root: &Path,
        commit: &str,
        previous: Option<&str>,
        notes: Vec<ChangeNote>,
    ) -> PinnedSdk {
        let dir = root.join(commit);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("surface.json"), r#"{"hash":"ab"}"#).unwrap();
        let build = sdk::Build {
            commit: commit.into(),
            game: sdk::GAME.into(),
            date: "2026-11-13".into(),
            nightly: None,
        };
        let mut index = Index::of_folder(&dir, build, "1.99.1".into()).unwrap();
        index.surface.previous = previous.map(str::to_owned);
        index.surface.notes = notes;
        std::fs::write(dir.join(sdk::INDEX), index.to_json()).unwrap();
        PinnedSdk {
            build: commit.into(),
            name: format!("name-{}", &commit[..1]),
            surface: dir.join("surface.json"),
            dir,
        }
    }

    /// Three builds, each carrying one note, the third (`c`) the newest.
    fn three_builds(root: &Path) -> Vec<PinnedSdk> {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let c = "c".repeat(40);
        vec![
            chained(root, &a, None, vec![note(&["hook:one"], "first", None)]),
            chained(
                root,
                &b,
                Some(&a),
                vec![note(&["hook:two"], "second", None)],
            ),
            chained(
                root,
                &c,
                Some(&b),
                vec![note(&["hook:three"], "third", None)],
            ),
        ]
    }

    /// Fetches the `sdk.json` of a build of `builds` by its id, as a release would.
    fn fetcher(builds: &[PinnedSdk]) -> impl Fn(&str) -> Result<Index, String> + '_ {
        |text| {
            let build = builds
                .iter()
                .find(|b| b.build == text)
                .ok_or_else(|| format!("no release for {text}"))?;
            let bytes = std::fs::read(build.dir.join(sdk::INDEX)).unwrap();
            Ok(Index::read(&bytes, &build.build).unwrap())
        }
    }

    fn why_by_build(chain: &sdk_notes::Chain) -> Vec<(&str, &str)> {
        chain
            .notes
            .iter()
            .map(|s| (s.build.as_str(), s.note.why.as_str()))
            .collect()
    }

    #[test]
    fn the_chain_collects_each_hop_and_stops_at_the_older_build() {
        let dir = tempfile::tempdir().unwrap();
        let builds = three_builds(dir.path());
        let chain = sdk_notes::chain(&builds[2], Some(&builds[0].build), fetcher(&builds)).unwrap();
        assert_eq!(
            why_by_build(&chain),
            [("name-c", "third"), ("bbbbbbb", "second")]
        );
        assert_eq!(chain.stopped, None);
    }

    #[test]
    fn the_chain_stops_at_a_build_named_twice() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = ("a".repeat(40), "b".repeat(40));
        let builds = [
            chained(
                dir.path(),
                &a,
                Some(&b),
                vec![note(&["hook:one"], "first", None)],
            ),
            chained(
                dir.path(),
                &b,
                Some(&a),
                vec![note(&["hook:two"], "second", None)],
            ),
        ];
        let chain = sdk_notes::chain(&builds[0], None, fetcher(&builds)).unwrap();
        assert_eq!(
            why_by_build(&chain),
            [("name-a", "first"), ("bbbbbbb", "second")]
        );
        assert!(
            chain.stopped.is_some_and(|why| why.contains("aaaaaaa")),
            "names the build named twice"
        );
    }

    #[test]
    fn the_chain_from_the_build_last_compiled_against_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let builds = three_builds(dir.path());
        let chain = sdk_notes::chain(&builds[2], Some(&builds[2].build), |text| {
            panic!("nothing to fetch, asked for {text}")
        })
        .unwrap();
        assert_eq!(chain.notes, []);
        assert_eq!(chain.stopped, None);
    }

    #[test]
    fn the_last_compiled_build_reads_back_and_anything_else_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(sdk_notes::last_built(dir.path()), None);
        std::fs::create_dir(dir.path().join(".sunrise")).unwrap();
        let build = "d".repeat(40);
        sdk_notes::record_built(dir.path(), &build).unwrap();
        assert_eq!(sdk_notes::last_built(dir.path()), Some(build));
        std::fs::write(dir.path().join(".sunrise/built"), "dddd\n").unwrap();
        assert_eq!(sdk_notes::last_built(dir.path()), None);
    }

    #[test]
    fn the_chain_without_an_older_build_ends_where_it_ends() {
        let dir = tempfile::tempdir().unwrap();
        let builds = three_builds(dir.path());
        let chain = sdk_notes::chain(&builds[2], None, fetcher(&builds)).unwrap();
        assert_eq!(
            why_by_build(&chain),
            [
                ("name-c", "third"),
                ("bbbbbbb", "second"),
                ("aaaaaaa", "first")
            ]
        );
        assert_eq!(chain.stopped, None);
    }

    #[test]
    fn the_chain_keeps_its_notes_when_a_build_cannot_be_fetched() {
        let dir = tempfile::tempdir().unwrap();
        let builds = three_builds(dir.path());
        let chain = sdk_notes::chain(&builds[2], None, |text| {
            Err(format!("no release for {text}"))
        })
        .unwrap();
        assert_eq!(why_by_build(&chain), [("name-c", "third")]);
        assert!(
            chain.stopped.is_some_and(|why| why.contains("bbbbbbb")),
            "names the build it stopped at"
        );
    }

    /// The names `explaining` matched for `ids` removed from a surface that still holds
    /// `hook:kept`, against a source that calls `gone_hook` and `kept`.
    fn explained_names(ids: &[&str]) -> Vec<String> {
        let notes = [shipped(
            "42c5f70",
            note(ids, "It moved.", Some("Use hook:new.")),
        )];
        let current = surface(vec![hook("hook:kept", "", &[])]);
        let sources = [(
            "src/lib.rs".to_owned(),
            "fn setup() { host.gone_hook(); // lost_hook\n let _ = kept; }".to_owned(),
        )];
        sdk_notes::explaining(&notes, &current, &sources)
            .into_iter()
            .flat_map(|e| e.names.into_iter().map(str::to_owned))
            .collect()
    }

    #[test]
    fn a_removed_id_the_source_names_is_explained() {
        assert_eq!(explained_names(&["hook:gone_hook"]), ["gone_hook"]);
    }

    #[test]
    fn a_removed_id_the_source_does_not_name_is_not_explained() {
        // `lost_hook` is only in a comment, which is no identifier.
        assert!(explained_names(&["hook:lost_hook", "hook:other"]).is_empty());
    }

    #[test]
    fn an_id_still_in_the_surface_is_not_explained() {
        assert!(explained_names(&["hook:kept"]).is_empty());
    }

    #[test]
    fn the_notes_follow_the_error_and_nothing_follows_when_none_matches() {
        let notes = [shipped(
            "42c5f70",
            note(&["hook:gone_hook"], "It moved.", Some("Use hook:new.")),
        )];
        let current = surface(vec![]);
        let sources = [(
            "src/lib.rs".to_owned(),
            "fn f() { gone_hook(); }".to_owned(),
        )];
        let chain = sdk_notes::Chain {
            notes: vec![],
            stopped: Some("build bbbbbbb: gone".into()),
        };
        let explained = sdk_notes::explaining(&notes, &current, &sources);
        let text = sdk_notes::with_notes("error: cargo failed".into(), &explained, &chain);
        assert!(text.starts_with("error: cargo failed\n"), "{text}");
        assert!(text.contains("`gone_hook`"), "{text}");
        assert!(text.contains("changed in 42c5f70"), "{text}");
        assert!(text.contains("Use hook:new."), "{text}");
        assert!(text.contains("bbbbbbb"), "{text}");
        // With no note, the line that says how to read older notes still follows.
        let text = sdk_notes::with_notes("error: cargo failed".into(), &[], &chain);
        assert!(text.starts_with("error: cargo failed\n"), "{text}");
        assert!(
            text.contains("bbbbbbb") && !text.contains("explain"),
            "{text}"
        );
        let whole = sdk_notes::Chain {
            notes: vec![],
            stopped: None,
        };
        assert_eq!(
            sdk_notes::with_notes("error: cargo failed".into(), &[], &whole),
            "error: cargo failed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_last_compiled_record_is_replaced_and_its_target_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere");
        std::fs::write(&target, "kept").unwrap();
        std::fs::create_dir(dir.path().join(".sunrise")).unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join(".sunrise/built")).unwrap();
        let build = "d".repeat(40);
        sdk_notes::record_built(dir.path(), &build).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "kept");
        assert_eq!(sdk_notes::last_built(dir.path()), Some(build));
    }

    fn argv(line: &str) -> Vec<String> {
        line.split(' ')
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn hooks_reads_its_options_and_refuses_others() {
        assert_eq!(
            hooks_args(&argv("block break --all")),
            Ok(("block break".into(), true))
        );
        let error = hooks_args(&argv("spawn --al")).unwrap_err();
        assert!(error.contains("--al"), "{error}");
        // The usage below the diagnostic has dashes of its own: the diagnostic names the separator.
        let error = hooks_args(&argv("block -- break")).unwrap_err();
        let refused = error.lines().next().unwrap().split(' ').next_back();
        assert_eq!(refused, Some("--"), "{error}");
    }
}
