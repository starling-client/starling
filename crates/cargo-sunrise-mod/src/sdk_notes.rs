//! The change notes of the SDK builds between two builds, and the ones that explain a failed
//! build of a mod. Each build's `sdk.json` carries the notes of the changes since the build
//! before it (`surface.previous`), so a mod that moves across several builds follows that chain.
//! Only each older build's signed `sdk.json` is fetched
//! ([`crate::sdk_fetch::fetch_index_only`]).
use crate::sdk_fetch::PinnedSdk;
use mod_surface::scan::{Token, tokens};
use mod_surface::{Surface, leaf};
use std::collections::BTreeSet;
use std::path::Path;
use sunrise_mod_package::sdk::{self, ChangeNote, Index};

/// The build the project's native mod last compiled against, written by `check` after a
/// compile succeeds: a later failed compile reads the notes since it.
const BUILT: &str = ".sunrise/built";

/// The build the project last compiled against, when `check` recorded one.
pub(crate) fn last_built(project: &Path) -> Option<String> {
    let bytes =
        crate::bounded::read_bytes(&project.join(BUILT), 64, crate::bounded::Symlinks::Refuse)
            .ok()?;
    let build = String::from_utf8(bytes).ok()?.trim().to_owned();
    sunrise_mod_package::manifest::revision(&build).then_some(build)
}

/// Record `build` as the one the project last compiled against.
pub(crate) fn record_built(project: &Path, build: &str) -> Result<(), String> {
    crate::internals::check_output_dir(&project.join(".sunrise"))?;
    // A link committed at `.sunrise/built` is replaced, never written through.
    crate::internals::atomic_write(&project.join(BUILT), format!("{build}\n"))
}

/// A change note and the build whose `sdk.json` carried it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Shipped {
    /// The build's name, as people read it.
    pub build: String,
    pub note: ChangeNote,
}

/// The notes found walking back through the builds, newest build first.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Chain {
    pub notes: Vec<Shipped>,
    /// Why the walk ended before the older build or the chain's end: the build it could not
    /// read, and why.
    pub stopped: Option<String>,
}

impl Chain {
    /// The line that says why the walk ended early, when it did.
    pub(crate) fn stopped_line(&self) -> Option<String> {
        self.stopped
            .as_ref()
            .map(|why| format!("Older change notes were not read: {why}"))
    }
}

/// `sdk`'s verified `sdk.json`.
fn index_of(sdk: &PinnedSdk) -> Result<Index, String> {
    let path = sdk.dir.join(sdk::INDEX);
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Index::read(&bytes, &sdk.build).map_err(|e| format!("{}: {e}", path.display()))
}

/// The notes of `newer` and of every build before it, following `surface.previous`: up to the
/// build `older` names (the walk stops when the chain reaches it, and its own notes are not
/// included; `older` naming `newer` gives no notes), or with no `older` to the chain's end. A
/// build `fetch` cannot read ends the walk with the notes found so far and the reason.
pub(crate) fn chain(
    newer: &PinnedSdk,
    older: Option<&str>,
    mut fetch: impl FnMut(&str) -> Result<Index, String>,
) -> Result<Chain, String> {
    let mut notes = Vec::new();
    let mut stopped = None;
    if older == Some(newer.build.as_str()) {
        return Ok(Chain { notes, stopped });
    }
    let mut name = newer.name.clone();
    let mut index = index_of(newer)?;
    // Each build names an older one; one named twice would send the walk round for ever.
    let mut seen = BTreeSet::from([newer.build.clone()]);
    loop {
        notes.extend(index.surface.notes.drain(..).map(|note| Shipped {
            build: name.clone(),
            note,
        }));
        let Some(previous) = index.surface.previous.take() else {
            break;
        };
        if older == Some(previous.as_str()) {
            break;
        }
        if !seen.insert(previous.clone()) {
            let short = previous.get(..7).unwrap_or(&previous);
            stopped = Some(format!("build {short} is named twice on the chain"));
            break;
        }
        match fetch(&previous) {
            Ok(next) => (name, index) = (crate::sdk_fetch::name(&next), next),
            Err(why) => {
                let short = previous.get(..7).unwrap_or(&previous);
                stopped = Some(format!("build {short}: {why}"));
                break;
            }
        }
    }
    Ok(Chain { notes, stopped })
}

/// A note that explains a failed build, with the ids of it the mod's source names.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Explained<'a> {
    pub names: Vec<&'a str>,
    pub shipped: &'a Shipped,
}

/// The notes of `notes` that name an id `surface` no longer holds whose final name occurs as an
/// identifier in `sources` (file and text), each once, with those names.
pub(crate) fn explaining<'a>(
    notes: &'a [Shipped],
    surface: &Surface,
    sources: &[(String, String)],
) -> Vec<Explained<'a>> {
    let present: BTreeSet<&str> = surface.entries().map(|(id, _)| id).collect();
    let written: BTreeSet<&str> = sources
        .iter()
        .flat_map(|(_, text)| tokens(text))
        .filter_map(|(token, _)| match token {
            Token::Ident(name) => Some(name),
            _ => None,
        })
        .collect();
    notes
        .iter()
        .filter_map(|shipped| {
            let mut names: Vec<&str> = shipped
                .note
                .ids
                .iter()
                .filter(|id| !present.contains(id.as_str()))
                .map(|id| leaf(id))
                .filter(|name| written.contains(name))
                .collect();
            names.sort_unstable();
            names.dedup();
            (!names.is_empty()).then_some(Explained { names, shipped })
        })
        .collect()
}

/// `error` followed by the notes that explain it and, after them, why the chain stopped early,
/// which says how to read the older notes. With neither, `error` as it is.
pub(crate) fn with_notes(error: String, explained: &[Explained<'_>], chain: &Chain) -> String {
    if explained.is_empty() && chain.stopped.is_none() {
        return error;
    }
    let mut text = format!("{error}\n\n");
    if !explained.is_empty() {
        text.push_str("Change notes that explain it:\n");
    }
    for item in explained {
        let names: Vec<String> = item.names.iter().map(|name| format!("`{name}`")).collect();
        text.push_str(&format!(
            "  {}: changed in {}\n      why: {}\n",
            names.join(", "),
            item.shipped.build,
            item.shipped.note.why
        ));
        if let Some(migrate) = &item.shipped.note.migrate {
            text.push_str(&format!("      migrate: {migrate}\n"));
        }
    }
    if let Some(line) = chain.stopped_line() {
        text.push_str(&line);
        text.push('\n');
    }
    text
}
