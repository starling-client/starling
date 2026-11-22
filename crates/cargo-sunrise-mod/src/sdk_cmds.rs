//! The commands that read the SDK's data (design §4, design-fit.md 10.2): `check` of a mod
//! folder, `hooks <text>`, `sdk diff <a> <b>`, `fits` and the report `update` prints before it
//! repins. Each reads the pinned build's `surface.json` from `.sunrise/sdk/`, or another build's
//! small files through [`crate::sdk_fetch::fetch_build`]; none links the generator's catalog.
use crate::options::Options;
use crate::sdk_fetch::{PinnedSdk, ensure_sdk, fetch_build, fetch_pinned};
use crate::uses_table::{self, Scanned, surface_of};
use mod_surface::child::ChildEntry;
use mod_surface::uses::{Change, Fit, FitReport, PLAIN_SCHEMA, USES_FILE, UsesTable};
use mod_surface::{Surface, leaf};
use std::path::Path;
use std::process::{Command, ExitCode};
use sunrise_mod_package::{
    Manifest, Package,
    manifest::MAX_USES,
    sdk::{self, ChangeNote, Index},
};
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

/// `sdk diff <a> <b>`: what changed in the surface from build `a` to build `b` (Nightly names
/// or shas). Only each build's small files are read, so a pruned build still answers.
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
    let notes = build_notes(&new)?;
    println!("{} to {}", old.name, new.name);
    print!(
        "{}",
        diff_text(&surface_of(&old)?, &surface_of(&new)?, &new.name, &notes)
    );
    Ok(())
}

/// The diff from `old` to `new`, with the change notes of the build `new` (named `new_name`)
/// under the changes they name: the notes of the changes since the SDK published before it.
fn diff_text(old: &Surface, new: &Surface, new_name: &str, notes: &[ChangeNote]) -> String {
    let mut diff = mod_surface::diff(old, new);
    for note in notes {
        let text = mod_surface::diff::Note {
            file: new_name.to_owned(),
            why: note.why.clone(),
            migrate: note.migrate.clone(),
        };
        // An id the note names may be unchanged here when `old` is not the build `new`
        // was compared with; the note then explains a change this diff does not show.
        diff.attach(&text, &note.ids);
    }
    diff.to_text()
}

/// What `fits` judges: the package's or the folder's table, or why the question does not apply.
pub(crate) enum Subject {
    Table {
        name: String,
        table: UsesTable,
        /// The mod's manifest: the game code it declares is judged with the list.
        manifest: Box<Manifest>,
    },
    /// The answer to print with exit status 2, and a reason suitable for the index.
    NotApplicable {
        message: String,
        reason: &'static str,
    },
}

pub(crate) struct PackageSubject {
    pub content_api: String,
    pub uses: Result<Subject, String>,
}

/// The manifest API requirement is known even when the optional uses list cannot be read.
pub(crate) fn package_subject(path: &Path) -> Result<PackageSubject, String> {
    let package = Package::read(path)?;
    Ok(PackageSubject {
        content_api: package.manifest.api.clone(),
        uses: package_uses(path, package),
    })
}

fn package_uses(path: &Path, package: Package) -> Result<Subject, String> {
    let name = &package.manifest.name;
    if let Some(table) = game_uses::unjudged(&package.manifest) {
        return Ok(Subject::NotApplicable {
            message: exact_text(name, &format!("it has a {table} table")),
            reason: table,
        });
    }
    let Some(native) = &package.manifest.native else {
        return Ok(Subject::NotApplicable {
            message: content_text(name),
            reason: "no native code",
        });
    };
    let pin = &native.sunrise;
    let Some(bytes) = Package::entry(path, USES_FILE, MAX_USES)? else {
        return Ok(Subject::NotApplicable {
            message: format!(
                "{name} pins build {pin} and its package carries no list of what it uses, so it can't be shown to fit another build. Pack it with `cargo sunrise-mod build`, which writes the list when the mod's code can be read, or repin it with `cargo sunrise-mod update --sunrise <build>` first."
            ),
            reason: "no uses list",
        });
    };
    let table = UsesTable::read(&bytes).map_err(|e| format!("{USES_FILE}: {e}"))?;
    if *pin != table.made_for {
        return Ok(Subject::NotApplicable {
            message: format!(
                "{name} pins build {pin} and its list of what it uses was made for {}, so the list says nothing about the pinned build. Pack it again with `cargo sunrise-mod build`.",
                table.made_for
            ),
            reason: "the uses list is for a different build",
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
            reason: "no native code",
        });
    };
    if let Some(table) = game_uses::unjudged(&parsed) {
        return Ok(Subject::NotApplicable {
            message: exact_text(name, &format!("it has a {table} table")),
            reason: table,
        });
    }
    let sdk = fetch_pinned(root, &native.sunrise)?;
    match uses_table::scan(entry, root, &parsed, &surface_of(&sdk)?)? {
        Scanned::Uses(uses) => Ok(Subject::Table {
            name: name.clone(),
            table: uses_table::table(&uses, root, &parsed, &sdk)?,
            manifest: Box::new(parsed),
        }),
        Scanned::Exact(why) => Ok(Subject::NotApplicable {
            message: exact_text(name, &why),
            reason: "the mod must be built for its pin",
        }),
        Scanned::Unread(why) => Err(why),
    }
}

fn content_text(name: &str) -> String {
    format!("{name} is a content mod, with no code to check against a build; `fits` doesn't apply.")
}

fn exact_text(name: &str, why: &str) -> String {
    format!("{name} is made for exactly one build ({why}), so `fits` doesn't apply.")
}

/// `fits [<file.smod>] [--build <name>]…`: one line per build (the current Nightly unless
/// `--build` names others) saying whether the mod fits it. Exits 0 when every build fits, 1 when
/// one does not, and 2 when the question cannot be answered or does not apply.
pub(crate) fn fits(entry: ChildEntry, args: &[String]) -> ExitCode {
    match fits_lines(entry, args) {
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

/// What `fits` answered, once its lines are printed.
enum Answer {
    Fits,
    DoesNotFit,
    /// No build misfits, and for one the list cannot say; its line is printed.
    Unknown,
    /// The question does not apply: why, not yet printed.
    NotApplicable(String),
}

/// `fits`'s package and builds, as given.
fn fits_args(args: &[String]) -> Result<(Option<&Path>, Vec<&str>), String> {
    let mut options = Options::new("fits", args, &["--build"]);
    let builds = options.values("--build")?;
    let package = options.at_most_one()?.map(Path::new);
    options.finish()?;
    Ok((package, builds))
}

fn fits_lines(entry: ChildEntry, args: &[String]) -> Result<Answer, String> {
    let (package, mut builds) = fits_args(args)?;
    if builds.is_empty() {
        builds.push("nightly");
    }
    let project = Path::new(".");
    let subject = match package {
        Some(path) => package_subject(path)?.uses?,
        None => folder_subject(entry, project)?,
    };
    let (name, mut table, manifest) = match subject {
        Subject::Table {
            name,
            table,
            manifest,
        } => (name, table, manifest),
        Subject::NotApplicable { message, .. } => return Ok(Answer::NotApplicable(message)),
    };
    // Whether the build the list was made for gave the hook versions a list of an older tool
    // lacks: asked for once, and only when a target leaves nothing else to judge.
    let mut baseline: Option<Result<(), String>> = None;
    println!("{name}:");
    let (mut all, mut known) = (true, true);
    for build in builds {
        let target = fetch_build(project, build)?;
        if target.build == table.made_for {
            println!("{}  made for", target.name);
            continue;
        }
        let target_surface = surface_of(&target)?;
        let game = game_fit(project, &manifest, &table, &target)?;
        let Comparison {
            report,
            mut compared,
            uncompared,
            unfetched,
        } = compare_fit(&mut table, &target_surface, game, &mut baseline, |pin| {
            let made_for = fetch_build(project, pin)?;
            std::fs::read(&made_for.surface)
                .map_err(|e| format!("{}: {e}", made_for.surface.display()))
        });
        let mut answer = report.verdict;
        if !compared.misfits.is_empty() {
            let mut misfits = match answer {
                Fit::DoesNotFit(misfits) => misfits,
                Fit::Fits | Fit::Unversioned(_) => Vec::new(),
            };
            misfits.append(&mut compared.misfits);
            misfits.sort();
            answer = Fit::DoesNotFit(misfits);
        }
        // What is already known to have changed is said, whatever the build data would add; with
        // nothing known, a comparison that could not be made is the answer.
        if let Some(why) = &uncompared
            && !matches!(answer, Fit::DoesNotFit(_))
        {
            return Err(why.clone());
        }
        match answer {
            Fit::Fits if !compared.unjudged.is_empty() => {
                known = false;
                println!(
                    "{}  not known: nothing the list can judge changed, but the build's data cannot show {} unchanged (it holds no body for a function the mod calls, or not every part of a loop).",
                    target.name,
                    compared
                        .unjudged
                        .iter()
                        .map(|id| format!("`{}`", id.rsplit(':').next().unwrap_or(id)))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Fit::Fits if compared.unrecorded => {
                known = false;
                println!(
                    "{}  not known: nothing the list can judge changed, but it holds no record of game code the mod declares. Pack the mod again with `cargo sunrise-mod build`, which records it.",
                    target.name
                );
            }
            Fit::Fits => println!("{}  fits", target.name),
            Fit::Unversioned(hooks) => {
                known = false;
                println!(
                    "{}  not known: nothing the list can judge changed, but it holds no version of {}{}. Pack the mod again with `cargo sunrise-mod build`, which records them.",
                    target.name,
                    hooks
                        .iter()
                        .map(|id| format!("`{}`", leaf(id)))
                        .collect::<Vec<_>>()
                        .join(", "),
                    unfetched.map_or(String::new(), |why| format!(
                        ", and the build it was made for could not be read for them ({why})"
                    ))
                );
            }
            Fit::DoesNotFit(misfits) => {
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
        if let Some(note) = report.deferred {
            println!("    {note}");
        }
    }
    Ok(match (all, known) {
        (false, _) => Answer::DoesNotFit,
        (true, false) => Answer::Unknown,
        (true, true) => Answer::Fits,
    })
}

pub(crate) struct Comparison {
    pub report: FitReport,
    pub compared: game_uses::Compared,
    uncompared: Option<String>,
    unfetched: Option<String>,
}

/// The mod API's items are judged by the surface, and declared game code by the build data.
/// Both the CLI and the registry compare them here; their callers supply the files.
pub(crate) fn compare_fit<B: AsRef<[u8]>>(
    table: &mut UsesTable,
    target_surface: &Surface,
    (compared, uncompared): (game_uses::Compared, Option<String>),
    baseline: &mut Option<Result<(), String>>,
    baseline_surface: impl FnOnce(&str) -> Result<B, String>,
) -> Comparison {
    let api = |table: &UsesTable| {
        table.fits(
            table
                .entries
                .iter()
                .filter(|entry| !entry.id.starts_with(game_uses::PREFIX)),
            target_surface,
        )
    };
    let mut report = api(table);
    let mut unfetched = None;
    if matches!(report.verdict, Fit::Unversioned(_)) && table.schema < PLAIN_SCHEMA {
        let fetched = baseline.get_or_insert_with(|| {
            let versions = baseline_surface(&table.made_for)?;
            table
                .pin_hook_versions(versions.as_ref())
                .map_err(|e| e.to_string())
        });
        match fetched {
            Ok(()) => report = api(table),
            Err(why) => unfetched = Some(why.clone()),
        }
    }
    Comparison {
        report,
        compared,
        uncompared,
        unfetched,
    }
}

/// The game code `manifest` declares, as `target` has it, against `table`'s record of it, and
/// the horizon-mixins revision the mod pins against the one `target`'s tool builds with. A
/// misfit already known stays when the build data cannot be read: why it cannot comes second.
fn game_fit(
    project: &Path,
    manifest: &Manifest,
    table: &UsesTable,
    target: &PinnedSdk,
) -> Result<(game_uses::Compared, Option<String>), String> {
    let path = target.dir.join(sdk::INDEX);
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let index =
        Index::read(&bytes, &target.build).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(compare_game(manifest, table, &index.mixins.rev, || {
        crate::sdk_fetch::build_data(project, &target.build)
            .and_then(|data| game_uses::entries(manifest, &data, &target.build))
    }))
}

pub(crate) fn compare_game(
    manifest: &Manifest,
    table: &UsesTable,
    mixins_rev: &str,
    target_entries: impl FnOnce() -> Result<Vec<(String, game_uses::Now)>, String>,
) -> (game_uses::Compared, Option<String>) {
    let mut compared = game_uses::Compared::default();
    let mut uncompared = None;
    if game_uses::declared(manifest) {
        let recorded: Vec<_> = table
            .entries
            .iter()
            .filter(|entry| entry.id.starts_with(game_uses::PREFIX))
            .collect();
        match target_entries() {
            Ok(now) => compared = game_uses::compare(&recorded, now),
            Err(why) => uncompared = Some(why),
        }
    }
    compared
        .misfits
        .extend(game_uses::tool(manifest, mixins_rev));
    (compared, uncompared)
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
/// `None`) breaks in this mod, item by item with the lines that use it; printed before `update`
/// repins. A mod made for exactly one build is only named.
pub(crate) fn update_report(
    entry: ChildEntry,
    root: &Path,
    to: Option<&str>,
) -> Result<(), String> {
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
    let old_surface = surface_of(&old)?;
    let uses = match uses_table::scan(entry, root, &parsed, &old_surface)? {
        Scanned::Uses(uses) => uses,
        Scanned::Exact(_) => {
            exact_line();
            return Ok(());
        }
        Scanned::Unread(why) => return Err(why),
    };
    let new = fetch_build(root, to.unwrap_or("nightly"))?;
    let new_surface = surface_of(&new)?;
    let notes = migrate_notes(&build_notes(&new)?);
    print!(
        "{}",
        report(name, &old, &new, &uses, &old_surface, &new_surface, &notes)
    );
    // The game code the mod declares, by the new build's build data.
    let made = uses_table::table(&uses, root, &parsed, &old)?;
    let (compared, uncompared) = game_fit(root, &parsed, &made, &new)?;
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

/// The change notes `sdk`'s `sdk.json` carries (`surface.notes`): those of the changes since the
/// SDK published before it. The fetch verified the file.
fn build_notes(sdk: &PinnedSdk) -> Result<Vec<ChangeNote>, String> {
    let path = sdk.dir.join(sdk::INDEX);
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let index = Index::read(&bytes, &sdk.build).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(index.surface.notes)
}

/// Each changed id's `migrate` line from `notes`; a note without one adds nothing.
fn migrate_notes(notes: &[ChangeNote]) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for note in notes {
        let Some(migrate) = &note.migrate else {
            continue;
        };
        for id in &note.ids {
            out.insert(id.clone(), migrate.clone());
        }
    }
    out
}

/// The text of [`update_report`].
fn report(
    name: &str,
    old: &PinnedSdk,
    new: &PinnedSdk,
    uses: &mod_surface::uses::Uses,
    old_surface: &Surface,
    new_surface: &Surface,
    notes: &std::collections::BTreeMap<String, String>,
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
        if let Some(migrate) = notes.get(id) {
            text.push_str(&format!("      note from the change: \"{migrate}\"\n"));
        }
    }
    text
}

/// `check [--uses]` in a mod folder: the package rules, then `cargo check` and an API use
/// report for a native mod. `--uses` lists each item and the line that brought it in.
pub(crate) fn check_mod(entry: ChildEntry, show_uses: bool) -> Result<(), String> {
    let root = Path::new(".");
    let package = Package::read(root)?;
    let files = package.directory_files(root)?;
    let (pipelines, refused) = crate::check::pipeline_shaders(&files);
    if !refused.is_empty() {
        return Err(format!(
            "render pipeline shaders refused:\n{}",
            refused.join("\n")
        ));
    }
    if pipelines > 0 {
        println!(
            "{pipelines} render pipeline shader(s) compose and validate as the game does \
             (a file that calls pipeline_texture is checked as textured, any other as untextured)."
        );
    }
    crate::check::references(root, &package.manifest)?;
    let Some(native) = &package.manifest.native else {
        println!("{}: the package rules hold.", package.manifest.name);
        return Ok(());
    };
    // The mod's `sunrise-mod-api` is a path into `.sunrise/sdk/`: fetch it before Cargo looks.
    let sdk = fetch_pinned(root, &native.sunrise)?;
    let surface = surface_of(&sdk)?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .arg("check")
        .current_dir(root)
        .status()
        .map_err(|e| format!("cargo check: {e}"))?;
    if !status.success() {
        let mut error = format!("cargo check failed: {status}");
        let required: Vec<_> = package
            .manifest
            .dependencies
            .iter()
            .filter_map(|(id, dependency)| (!dependency.details().optional).then_some(id.as_str()))
            .collect();
        if !required.is_empty() {
            error.push_str(&format!(
                "\nRequired mods: {}. A standalone build does not supply library mods.",
                required.join(", ")
            ));
        }
        return Err(error);
    }
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
        package_subject(&path).unwrap().uses.unwrap()
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

    #[test]
    fn a_content_mods_update_report_does_not_access_the_sdk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(MANIFEST),
            "id = \"greeter\"\nname = \"Greeter\"\nversion = \"0.1.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n",
        )
        .unwrap();
        assert_eq!(
            update_report(mod_surface::child::child_entry(), dir.path(), None),
            Ok(())
        );
        assert!(!dir.path().join(".sunrise").exists());
    }

    #[test]
    fn the_update_report_takes_each_ids_migrate_line() {
        let notes = migrate_notes(&[
            note(
                &["method:ServerHost::give_item"],
                "Parts",
                Some("items go through Parts under a transaction"),
            ),
            note(&["hook:x"], "only why", None),
        ]);
        assert_eq!(
            notes
                .get("method:ServerHost::give_item")
                .map(String::as_str),
            Some("items go through Parts under a transaction")
        );
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn sdk_diff_prints_the_newer_builds_notes_under_the_changes_they_name() {
        let old = surface(vec![hook("hook:gone", "", &[])]);
        let new = surface(vec![]);
        let text = diff_text(
            &old,
            &new,
            "42c5f70",
            &[note(
                &["hook:gone"],
                "It fired twice.",
                Some("Listen to hook:once."),
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

    fn argv(line: &str) -> Vec<String> {
        line.split(' ')
            .filter(|w| !w.is_empty())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn hooks_fits_and_check_read_their_options_and_refuse_others() {
        assert_eq!(
            hooks_args(&argv("block break --all")),
            Ok(("block break".into(), true))
        );
        let build = argv("pack.smod --build a --build b");
        assert_eq!(
            fits_args(&build),
            Ok((Some(Path::new("pack.smod")), vec!["a", "b"]))
        );
        for (error, option) in [
            (hooks_args(&argv("spawn --al")).unwrap_err(), "--al"),
            (
                fits_args(&argv("--bild a")).map(drop).unwrap_err(),
                "--bild",
            ),
            (
                fits_args(&argv("a.smod b.smod")).map(drop).unwrap_err(),
                "b.smod",
            ),
        ] {
            assert!(error.contains(option), "{error}");
        }
        // The usage below the diagnostic has dashes of its own: the diagnostic names the separator.
        let error = hooks_args(&argv("block -- break")).unwrap_err();
        let refused = error.lines().next().unwrap().split(' ').next_back();
        assert_eq!(refused, Some("--"), "{error}");
    }
}
