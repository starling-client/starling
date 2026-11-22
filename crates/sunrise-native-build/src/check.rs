//! How native mods stand against one game build before anything compiles: each mod's refusal and fit (SDK design §10), and the route the usable ones take.
use crate::{api, game_uses, prebuilt, scan_bundles};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use sunrise_mod_package::mod_surface::{
    self,
    uses::{self, Change, Fit, Misfit, UsesError, UsesTable},
};

/// How a mod stands against the build an instance plays.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Verdict {
    /// Made for this build.
    MadeFor,
    /// Made for another build (`made_for`), and every item it uses is unchanged in this one.
    Fits { made_for: String },
    /// It names game code no list can show unchanged (`why`: the `[native]` table whose rows no build data describes, or `internals` for a function whose body this build's data does not hold): it works on the build it was made for only.
    Exact { made_for: String, why: &'static str },
    /// Its package doesn't say enough about what it uses (`why`): it works on the build it was made for only.
    NoList {
        made_for: String,
        why: Unlisted,
        /// With [`Unlisted::HookVersions`]: the hooks whose version the list does not hold.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        hooks: Vec<String>,
    },
    /// Items it uses changed in this build.
    DoesNotFit {
        made_for: String,
        misfits: Vec<Misfit>,
    },
}

impl Verdict {
    /// Whether the mod can be built against this build.
    pub fn usable(&self) -> bool {
        matches!(self, Self::MadeFor | Self::Fits { .. })
    }
}

/// Why a package's list of what the mod uses cannot show that the mod fits another build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Unlisted {
    /// The package carries no list.
    Missing,
    /// Its list cannot be read.
    Unreadable,
    /// Its list has a schema newer than this build's tool reads: a newer game build reads it.
    NewerSchema,
    /// Its list was made for another build than the mod pins.
    OtherBuild,
    /// Nothing the list can judge changed, but it holds no version of some hook it names: packing the mod again writes them.
    HookVersions,
    /// Nothing the list can judge changed, but it holds no record of some game code the mod declares (a mixin's target, a loop, an internal): packing the mod again writes them.
    GameCode,
}

/// The uses table `dir` carries, or why it gives none this build reads; then the mod works on its own build only.
pub fn table(dir: &Path) -> Result<UsesTable, Unlisted> {
    let file = std::fs::File::open(dir.join(uses::USES_FILE)).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Unlisted::Missing
        } else {
            Unlisted::Unreadable
        }
    })?;
    let mut bytes = Vec::new();
    file.take(uses::MAX_USES_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Unlisted::Unreadable)?;
    UsesTable::read(&bytes).map_err(|error| match error {
        UsesError::Schema(schema) if schema > uses::USES_SCHEMA => Unlisted::NewerSchema,
        _ => Unlisted::Unreadable,
    })
}

/// How `project` stands against build `build`, whose surface `surface` gives (asked for only when the mod was made for another build), and where the game code the mod declares is as `game` says ([`game_uses::entries`], asked for only when it declares some).
pub fn judge<E>(
    project: &api::Project<'_>,
    build: &str,
    surface: &mut dyn FnMut() -> Result<Rc<mod_surface::Surface>, E>,
    game: &mut dyn FnMut() -> Result<Vec<(String, game_uses::Now)>, E>,
) -> Result<Verdict, E> {
    let made_for = project.bundle.native().sunrise.clone();
    if made_for == build {
        return Ok(Verdict::MadeFor);
    }
    if let Some(why) = game_uses::unjudged(&project.bundle.manifest) {
        return Ok(Verdict::Exact { made_for, why });
    }
    let unlisted = |why, hooks| Verdict::NoList {
        made_for: made_for.clone(),
        why,
        hooks,
    };
    let table = match table(&project.bundle.dir) {
        Ok(table) if table.made_for == made_for => table,
        Ok(_) => return Ok(unlisted(Unlisted::OtherBuild, Vec::new())),
        Err(why) => return Ok(unlisted(why, Vec::new())),
    };
    // The mod API's items are judged by the surface, the game code the mod declares by the build data: one list holds both.
    let (declared, api): (Vec<_>, Vec<_>) = table
        .entries
        .iter()
        .partition(|entry| entry.id.starts_with(game_uses::PREFIX));
    let (mut misfits, mut unversioned) = match table.fits(api, surface()?.as_ref()).verdict {
        Fit::Fits => (Vec::new(), Vec::new()),
        Fit::DoesNotFit(misfits) => (misfits, Vec::new()),
        // The build the list was made for, which has the versions, is not at hand here.
        Fit::Unversioned(hooks) => (Vec::new(), hooks),
    };
    // A build with this tool refuses a mod that pins another horizon-mixins revision than the tool's.
    misfits.extend(game_uses::tool(&project.bundle.manifest, crate::MIXINS_REV));
    let mut compared = game_uses::Compared::default();
    if game_uses::declared(&project.bundle.manifest) {
        match game() {
            Ok(now) => compared = game_uses::compare(&declared, now),
            // What is already known to have changed is said, whatever the build data would add.
            Err(_) if !misfits.is_empty() => {}
            Err(lack) => return Err(lack),
        }
    }
    misfits.append(&mut compared.misfits);
    if !misfits.is_empty() {
        misfits.sort();
        return Ok(Verdict::DoesNotFit { made_for, misfits });
    }
    // Code the build data cannot fingerprint is never shown unchanged.
    if let Some(id) = compared.unjudged.first() {
        let why = if id.starts_with("game:loop:") {
            "[[native.loop]]"
        } else {
            "internals"
        };
        return Ok(Verdict::Exact { made_for, why });
    }
    if compared.unrecorded {
        return Ok(unlisted(Unlisted::GameCode, Vec::new()));
    }
    if !unversioned.is_empty() {
        return Ok(unlisted(
            Unlisted::HookVersions,
            std::mem::take(&mut unversioned),
        ));
    }
    Ok(Verdict::Fits { made_for })
}

/// What `cargo sunrise-mod native-check` checks: the first line of standard input.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    /// Must be [`crate::contract::CONTRACT`].
    pub contract: u32,
    /// A mod's folder, or a folder of mod folders.
    pub mods: PathBuf,
    /// The build id (the game's full commit) the mods are checked against.
    pub build: String,
    /// The build's `surface.json`; absent, a mod that needs it says so (`needs`).
    #[serde(default)]
    pub surface: Option<PathBuf>,
    /// The build's unpacked build data (`functions.json`, `internals.json`, `loops.json`);
    /// absent, a mod that needs it says so. Same-build mixins need `functions.json` too.
    #[serde(default)]
    pub build_data: Option<PathBuf>,
}

/// One mod's standing.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Checked {
    pub id: String,
    /// Why no build takes it, as the build log would say; then it has no verdict.
    pub refusal: Option<String>,
    pub verdict: Option<Verdict>,
    /// The file the plan didn't give that its verdict needs: `surface` or `build-data`.
    pub needs: Option<&'static str>,
}

/// The reply, one line of standard output.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Reply {
    pub contract: u32,
    /// The route a build of the usable mods takes (`prebuilt-api` or `prebuilt`), none when none is usable.
    pub route: Option<&'static str>,
    pub mods: Vec<Checked>,
}

/// What prevented a fit verdict: missing or unreadable data, or a named refusal.
enum Lack {
    Needs(&'static str),
    Unread(String),
    Refused(String),
}

/// Checks `plan`'s mods against its build.
pub fn check(plan: &Plan) -> Result<Reply, String> {
    let found = scan_bundles(&plan.mods)?;
    let mut surface: Option<Rc<mod_surface::Surface>> = None;
    let mut functions = None;
    let mut mods = Vec::new();
    let mut usable = Vec::new();
    for bundle in &found {
        let id = bundle.manifest.id.clone();
        let project = match api::Project::read(bundle) {
            Ok(project) => project,
            Err(error) => {
                mods.push(Checked {
                    id,
                    refusal: Some(error.to_string()),
                    verdict: None,
                    needs: None,
                });
                continue;
            }
        };
        if let Some(refusal) = prebuilt::project_refusal(&project) {
            mods.push(Checked {
                id,
                refusal: Some(refusal.plain()),
                verdict: None,
                needs: None,
            });
            continue;
        }
        let native = bundle.native();
        let judged = if native.sunrise == plan.build && !native.mixin.is_empty() {
            (|| {
                let dir = plan.build_data.as_ref().ok_or(Lack::Needs("build-data"))?;
                let index = match &functions {
                    Some(index) => index,
                    None => functions
                        .insert(game_uses::function_index(dir, &plan.build).map_err(Lack::Unread)?),
                };
                for row in &native.mixin {
                    if let Some(refused) = game_uses::mixin_refusal(index, &row.target) {
                        return Err(Lack::Refused(format!(
                            "{}: {id}:{}: {}: {}",
                            refused.code.name(),
                            row.id,
                            row.target,
                            refused.why,
                        )));
                    }
                }
                Ok(Verdict::MadeFor)
            })()
        } else {
            judge(
                &project,
                &plan.build,
                &mut || {
                    if let Some(read) = &surface {
                        return Ok(read.clone());
                    }
                    let path = plan.surface.as_ref().ok_or(Lack::Needs("surface"))?;
                    let bytes = std::fs::read(path)
                        .map_err(|e| Lack::Unread(format!("{}: {e}", path.display())))?;
                    let read = Rc::new(
                        mod_surface::Surface::read(&bytes)
                            .map_err(|e| Lack::Unread(format!("{}: {e}", path.display())))?,
                    );
                    surface = Some(read.clone());
                    Ok(read)
                },
                &mut || {
                    let dir = plan.build_data.as_ref().ok_or(Lack::Needs("build-data"))?;
                    game_uses::entries(&bundle.manifest, dir, &plan.build).map_err(Lack::Unread)
                },
            )
        };
        match judged {
            Ok(mut verdict) => {
                if let Verdict::DoesNotFit { misfits, .. } = &mut verdict {
                    for misfit in misfits.iter_mut() {
                        if matches!(
                            misfit.change,
                            Change::Refused { .. }
                                | Change::BodyChanged { .. }
                                | Change::DeclarationChanged { .. }
                        ) {
                            // Contract 2 callers isolate unfit mods and their dependents, but
                            // only decode known changes. Carry the full diagnostic as entry
                            // text, not a `game:fn:` id the launcher shortens to its leaf.
                            misfit.id = misfit.to_string();
                            misfit.change = Change::Changed;
                        }
                    }
                    // A function declared as both a mixin target and an internal reads the same.
                    let mut seen = BTreeSet::new();
                    misfits.retain(|misfit| seen.insert(misfit.clone()));
                }
                if verdict.usable() {
                    usable.push(project);
                }
                mods.push(Checked {
                    id,
                    refusal: None,
                    verdict: Some(verdict),
                    needs: None,
                });
            }
            Err(Lack::Needs(what)) => mods.push(Checked {
                id,
                refusal: None,
                verdict: None,
                needs: Some(what),
            }),
            Err(Lack::Unread(why)) => return Err(why),
            Err(Lack::Refused(why)) => mods.push(Checked {
                id,
                refusal: Some(why),
                verdict: None,
                needs: None,
            }),
        }
    }
    Ok(Reply {
        contract: crate::contract::CONTRACT,
        route: (!usable.is_empty()).then(|| prebuilt::Route::of(&usable).name()),
        mods,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(mods: &Path, build: &str) -> Plan {
        Plan {
            contract: crate::contract::CONTRACT,
            mods: mods.to_owned(),
            build: build.into(),
            surface: None,
            build_data: None,
        }
    }

    #[test]
    fn mods_made_for_the_build_take_its_route_and_others_say_what_they_need() {
        let dir = crate::tests::dir("check-route");
        let project = dir.join("here");
        crate::tests::write_bundle(&project, "here", "");
        std::fs::write(
            project.join("Cargo.toml"),
            "[package]\nname='here'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}\n",
        )
        .unwrap();
        let reply = check(&plan(&dir, crate::tests::A)).unwrap();
        assert_eq!(reply.mods[0].refusal, None);
        assert_eq!(reply.mods.len(), 1);
        assert_eq!(reply.mods[0].verdict, Some(Verdict::MadeFor));
        assert!(reply.route.is_some());
        let elsewhere = check(&plan(&dir, "another-build")).unwrap();
        assert_eq!(
            elsewhere.mods[0].verdict,
            Some(Verdict::NoList {
                made_for: crate::tests::A.to_string(),
                why: Unlisted::Missing,
                hooks: Vec::new(),
            }),
            "no uses table: made for its own build only"
        );
        assert_eq!(elsewhere.route, None);
    }
}

#[cfg(test)]
#[path = "check_tests.rs"]
mod fit_tests;
