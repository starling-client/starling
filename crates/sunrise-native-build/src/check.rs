//! How native mods stand against one game build before anything compiles: each mod's refusal and fit (SDK design §10), and the route the usable ones take.
use crate::{api, game_uses, prebuilt, scan_bundles};
use serde::{Deserialize, Serialize};
use std::cell::OnceCell;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use sunrise_mod_package::Manifest;
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

/// What one build makes of a native mod: its verdict, and what the verdict leaves out.
#[derive(Clone, Debug, PartialEq)]
pub struct Judged<E> {
    pub verdict: Verdict,
    /// Entries of a list written before the surface's schema 3 that only the compile can judge ([`uses::FitReport::deferred`]): shown beside the verdict, never a refusal.
    pub deferred: Option<String>,
    /// With [`Verdict::DoesNotFit`]: why the game code the mod declares could not be compared too.
    pub uncompared: Option<E>,
    /// The ids of declared game code the build data cannot show unchanged: with nothing known to have changed, they make the verdict [`Verdict::Exact`].
    pub unjudged: Vec<String>,
    /// With [`Unlisted::HookVersions`]: why the build the mod was made for could not give the hook versions the list lacks.
    pub unpinned: Option<E>,
}

/// The builds a fit compares: the one a mod was made for, the one it is judged against, and the horizon-mixins revision that one's tool builds with.
#[derive(Clone, Copy, Debug)]
pub struct Builds<'a> {
    pub made_for: &'a str,
    pub build: &'a str,
    pub mixins_rev: &'a str,
}

/// How the native mod `manifest` stands against `builds.build`, by `table`, the list of what it uses (or why it has none this tool reads). `surface` gives the build's surface, and `game` where the game code the mod declares is in it ([`game_uses::entries`]). `made_for_surface`, when at hand, gives the `surface.json` of the build the mod was made for, whose hook versions complete a list an older tool wrote, into the table. Each is asked for only when the verdict needs it; `Err` is what `surface` or `game` returned, and then the verdict is not known. A made-for surface that cannot be had or read leaves the hooks unversioned, and says why in [`Judged::unpinned`].
pub fn judge<'t, E: From<String>>(
    manifest: &Manifest,
    builds: Builds<'_>,
    table: impl FnOnce() -> Result<&'t mut UsesTable, Unlisted>,
    surface: &mut dyn FnMut() -> Result<Rc<mod_surface::Surface>, E>,
    game: &mut dyn FnMut() -> Result<Vec<(String, game_uses::Now)>, E>,
    made_for_surface: Option<&mut dyn FnMut() -> Result<Vec<u8>, E>>,
) -> Result<Judged<E>, E> {
    let Builds {
        made_for,
        build,
        mixins_rev,
    } = builds;
    let made_for = made_for.to_owned();
    let verdict = |verdict| Judged {
        verdict,
        deferred: None,
        uncompared: None,
        unjudged: Vec::new(),
        unpinned: None,
    };
    if made_for == build {
        return Ok(verdict(Verdict::MadeFor));
    }
    if let Some(why) = game_uses::unjudged(manifest) {
        return Ok(verdict(Verdict::Exact { made_for, why }));
    }
    let table = match table() {
        Ok(table) if table.made_for == made_for => table,
        why => {
            let why = why.map_or_else(|why| why, |_| Unlisted::OtherBuild);
            return Ok(verdict(Verdict::NoList {
                made_for,
                why,
                hooks: Vec::new(),
            }));
        }
    };
    // The mod API's items are judged by the surface, the game code the mod declares by the build data: one list holds both.
    let surface = surface()?;
    let api = |table: &UsesTable| {
        let entries = table.entries.iter();
        table.fits(
            entries.filter(|entry| !entry.id.starts_with(game_uses::PREFIX)),
            &surface,
        )
    };
    let mut report = api(table);
    // A list an older tool wrote holds no hook versions, and the build it was made for has them: asked for whenever the list leaves a hook unjudged, whatever the game code shows, so a misfit they reveal is named beside the rest. A surface that gives none leaves the list as it was.
    let mut unpinned = None;
    if matches!(report.verdict, Fit::Unversioned(_))
        && table.schema < uses::PLAIN_SCHEMA
        && let Some(made_for_surface) = made_for_surface
    {
        let pinned = made_for_surface().and_then(|bytes| {
            table
                .pin_hook_versions(&bytes)
                .map_err(|e| E::from(e.to_string()))
        });
        match pinned {
            Ok(()) => report = api(table),
            Err(why) => unpinned = Some(why),
        }
    }
    let (mut compared, uncompared) = game_uses::compare_declared(manifest, table, mixins_rev, game);
    let mut misfits = match &mut report.verdict {
        Fit::DoesNotFit(misfits) => std::mem::take(misfits),
        Fit::Fits | Fit::Unversioned(_) => Vec::new(),
    };
    misfits.append(&mut compared.misfits);
    // What is already known to have changed is said, whatever the build data would add.
    if misfits.is_empty()
        && let Some(lack) = uncompared
    {
        return Err(lack);
    }
    let verdict = if !misfits.is_empty() {
        misfits.sort();
        Verdict::DoesNotFit { made_for, misfits }
    } else if let Some(id) = compared.unjudged.first() {
        // Code the build data cannot fingerprint is never shown unchanged.
        let why = if id.starts_with("game:loop:") {
            "[[native.loop]]"
        } else {
            "internals"
        };
        Verdict::Exact { made_for, why }
    } else if compared.unrecorded {
        Verdict::NoList {
            made_for,
            why: Unlisted::GameCode,
            hooks: Vec::new(),
        }
    } else if let Fit::Unversioned(hooks) = report.verdict {
        Verdict::NoList {
            made_for,
            why: Unlisted::HookVersions,
            hooks,
        }
    } else {
        Verdict::Fits { made_for }
    };
    Ok(Judged {
        verdict,
        deferred: report.deferred,
        uncompared,
        unjudged: compared.unjudged,
        unpinned,
    })
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
    /// The build's complete unpacked build data (`functions.json`, `internals.json`, `loops.json`).
    /// A mod that needs it reports `needs` if it is absent. Same-build mixins and loops need it too.
    /// When read, missing files or a wrong build are errors even if a misfit is already known.
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

impl From<String> for Lack {
    fn from(why: String) -> Self {
        Self::Unread(why)
    }
}

/// Checks `plan`'s mods against its build.
pub fn check(plan: &Plan) -> Result<Reply, String> {
    let found = scan_bundles(&plan.mods)?;
    let mut surface: Option<Rc<mod_surface::Surface>> = None;
    // One set of published inputs for this check, opened only if a verdict needs it.
    let build_data = OnceCell::new();
    let data = || {
        let dir = plan
            .build_data
            .as_deref()
            .ok_or(Lack::Needs("build-data"))?;
        build_data
            .get_or_init(|| game_uses::Data::read(dir, &plan.build))
            .as_ref()
            .map_err(|error| Lack::Unread(error.to_string()))
    };
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
        let judged = if native.sunrise == plan.build
            && (!native.mixin.is_empty() || !native.r#loop.is_empty())
        {
            data().and_then(|data| {
                for row in &native.mixin {
                    if let Some(refused) = data.mixin_refusal(&row.target) {
                        return Err(Lack::Refused(format!(
                            "{}: {id}:{}: {}: {}",
                            refused.code.name(),
                            row.id,
                            row.target,
                            refused.why,
                        )));
                    }
                }
                for row in &native.r#loop {
                    data.loops.require(&id, row).map_err(Lack::Refused)?;
                }
                Ok(Verdict::MadeFor)
            })
        } else {
            // Read only when the verdict needs it.
            let mut read = None;
            let read = &mut read;
            judge(
                &bundle.manifest,
                Builds {
                    made_for: &native.sunrise,
                    build: &plan.build,
                    mixins_rev: crate::MIXINS_REV,
                },
                move || read.insert(table(&bundle.dir)).as_mut().map_err(|why| *why),
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
                &mut || Ok(data()?.entries(&bundle.manifest)),
                // The plan gives no surface of the build the mod was made for.
                None,
            )
            .and_then(|judged| match judged.uncompared {
                Some(Lack::Unread(why)) => Err(Lack::Unread(why)),
                _ => Ok(judged.verdict),
            })
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
