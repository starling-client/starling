//! What a mod declares of the game's own code (`[[native.mixin]]` targets, `[[native.loop]]` loops and its `internals`), as entries of the list of what it uses: one function reads them from a build's build data, for the packer to record at the pinned build and for the fit check to compare at the installed one.
use std::path::Path;
use sunrise_mod_package::internals::{self, ListedInternal};
use sunrise_mod_package::mod_surface::uses::{Change, Entry, GameCode, Misfit};
use sunrise_mod_package::{
    Manifest,
    build_data::{self, ReadError},
};
use sunrise_platform::mod_build::api_gen::{
    self,
    functions::{self, Function, FunctionIndex},
};

/// What every such entry's id starts with; no surface item's id does.
pub const PREFIX: &str = "game:";

/// The `[native]` table whose rows name game code no build data describes, so a mod with one works on the build it was made for only; `None` for a mod a list can judge.
pub fn unjudged(manifest: &Manifest) -> Option<&'static str> {
    let native = manifest.native.as_ref()?;
    [
        (native.module.is_empty(), "[[native.module]]"),
        (native.bridge.is_empty(), "[[native.bridge]]"),
        (native.state.is_empty(), "[[native.state]]"),
    ]
    .into_iter()
    .find_map(|(empty, table)| (!empty).then_some(table))
}

/// Whether `manifest` declares game code [`entries`] lists.
pub fn declared(manifest: &Manifest) -> bool {
    !manifest.internals.is_empty()
        || manifest
            .native
            .as_ref()
            .is_some_and(|native| !native.mixin.is_empty() || !native.r#loop.is_empty())
}

/// A declared piece of game code in one build.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Now {
    /// Its fingerprint there.
    Is {
        hash: String,
        game_code: Option<GameCode>,
    },
    /// The build has no such item.
    Gone,
    /// The target is present, but the function index refuses to replace it.
    Refused { code: &'static str, why: String },
    /// The build has it, and its build data does not hold the code a fingerprint needs (the body of a function it does not index), so no list can show it unchanged.
    Unjudged,
}

impl Now {
    fn fingerprint<'a>(
        hash: String,
        loop_contract: Option<String>,
        functions: impl IntoIterator<Item = (&'a str, &'a Function)>,
    ) -> Self {
        let digests = functions
            .into_iter()
            .map(|(path, function)| Some((path.to_owned(), function.digests.clone()?)))
            .collect::<Option<_>>();
        Self::Is {
            hash,
            game_code: digests.map(|functions| GameCode {
                loop_contract,
                functions,
            }),
        }
    }

    /// The record the packer writes if this build can fingerprint the item.
    pub fn into_entry(self, id: String) -> Option<Entry> {
        match self {
            Self::Is { hash, game_code } => Some(Entry {
                game_code,
                ..Entry::plain(id, hash)
            }),
            Self::Gone | Self::Refused { .. } | Self::Unjudged => None,
        }
    }
}

/// The misfit of a mod that pins a horizon-mixins revision other than `revision`, the one the build's tool builds with: that build refuses the mod.
pub fn tool(manifest: &Manifest, revision: &str) -> Option<Misfit> {
    let native = manifest.native.as_ref()?;
    let pinned = native.mixins.as_deref()?;
    (!native.mixin.is_empty() && pinned != revision).then(|| Misfit {
        id: format!("{PREFIX}tool:horizon-mixins"),
        change: Change::Changed,
    })
}

/// The build-data file `file` of `dir`, which must describe `build`: a folder of another build's data would compare that build with itself.
fn document<T: serde::de::DeserializeOwned>(
    dir: &Path,
    file: &str,
    build: &str,
) -> Result<T, ReadError> {
    let path = dir.join(file);
    let invalid = |reason| ReadError::Invalid(format!("{}: {reason}", path.display()));
    let text = std::fs::read_to_string(&path).map_err(|e| invalid(e.to_string()))?;
    let document = build_data::read::<T>(file, &text);
    let described = match &document {
        Ok((described, _))
        | Err(ReadError::Schema {
            build: described, ..
        }) => described,
        Err(error) => return Err(invalid(error.to_string())),
    };
    if described.commit != build {
        return Err(invalid(format!(
            "it describes game build {}, not {build}",
            described.commit
        )));
    }
    document.map(|(_, body)| body)
}

pub(crate) fn function_index(dir: &Path, build: &str) -> Result<FunctionIndex, String> {
    document(dir, functions::FILE, build).map_err(|error| error.to_string())
}

pub(crate) fn mixin_refusal<'a>(
    index: &'a FunctionIndex,
    target: &str,
) -> Option<&'a functions::Refused> {
    index.refused.get(&functions::key(target))
}

/// Each piece of game code `manifest` declares, by id, as it is in `build`, whose build data is in `dir`. A function is fingerprinted by signature and body, since a replacement or a call written for an old body could meet a different one; a type by its signature; a loop by what the game publishes of it, its two functions and its slot trait.
pub fn entries(manifest: &Manifest, dir: &Path, build: &str) -> Result<Vec<(String, Now)>, String> {
    let loops = manifest
        .native
        .as_ref()
        .is_some_and(|n| !n.r#loop.is_empty());
    Ok(
        Data::read_for(dir, build, loops || !manifest.internals.is_empty(), loops)
            .map_err(|error| error.to_string())?
            .entries(manifest),
    )
}

/// The game-code tables of one build, read once when judging many packages.
pub struct Data {
    index: FunctionIndex,
    internals: Vec<ListedInternal>,
    loops: Vec<api_gen::loops::PublishedLoop>,
}

impl Data {
    /// Read all build-data files, including those not used by any currently listed mod.
    pub fn read(dir: &Path, build: &str) -> Result<Self, ReadError> {
        Self::read_for(dir, build, true, true)
    }

    fn read_for(dir: &Path, build: &str, internals: bool, loops: bool) -> Result<Self, ReadError> {
        #[derive(serde::Deserialize)]
        struct Loops {
            loops: Vec<api_gen::loops::PublishedLoop>,
        }
        #[derive(serde::Deserialize)]
        struct Internals {
            internals: Vec<ListedInternal>,
        }
        let index = document(dir, api_gen::functions::FILE, build);
        let internals = if internals {
            document::<Internals>(dir, build_data::INTERNALS, build).map(|doc| doc.internals)
        } else {
            Ok(Vec::new())
        };
        let loops = if loops {
            document::<Loops>(dir, api_gen::loops::FILE, build).and_then(|doc| {
                if let Some(l) = doc.loops.iter().find(|l| !l.entry.contains("::")) {
                    return Err(ReadError::Invalid(format!(
                        "{}: the loop `{}` has the entry `{}`, which is not a function's path",
                        dir.join(api_gen::loops::FILE).display(),
                        l.name,
                        l.entry
                    )));
                }
                Ok(doc.loops)
            })
        } else {
            Ok(Vec::new())
        };
        // A schema we do not read does not hide missing files, corrupt data, or a wrong build.
        match (index, internals, loops) {
            (Ok(index), Ok(internals), Ok(loops)) => Ok(Self {
                index,
                internals,
                loops,
            }),
            (Err(e @ ReadError::Invalid(_)), _, _)
            | (_, Err(e @ ReadError::Invalid(_)), _)
            | (_, _, Err(e @ ReadError::Invalid(_))) => Err(e),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
        }
    }

    /// The game code this mod declares, as this build has it.
    pub fn entries(&self, manifest: &Manifest) -> Vec<(String, Now)> {
        let native = manifest.native.as_ref();
        let mixins = native.map_or(&[][..], |native| &native.mixin);
        let loops = native.map_or(&[][..], |native| &native.r#loop);
        let index = &self.index;
        let code = |target: &str| index.functions.get(&functions::key(target));
        let span_hash = |f: &Function| format!("{}:{}", f.signature_hash, f.sha256);
        let listed = &self.internals;
        let mut entries = Vec::new();
        for row in mixins {
            entries.push((
                format!("{PREFIX}fn:{}", row.target),
                if let Some(refused) = mixin_refusal(index, &row.target) {
                    Now::Refused {
                        code: refused.code.name(),
                        why: refused.why.clone(),
                    }
                } else {
                    code(&row.target).map_or(Now::Gone, |f| {
                        Now::fingerprint(span_hash(f), None, [(row.target.as_str(), f)])
                    })
                },
            ));
        }
        for row in loops {
            let now = match self.loops.iter().find(|l| l.name == row.name) {
                None => Now::Gone,
                Some(l) => {
                    let generic = l.generic_target();
                    // The slot trait lies in the module of the loop's functions.
                    let slot = index
                        .functions
                        .get(&api_gen::functions::key(&generic))
                        .and_then(|f| {
                            let path = format!("{}::{}", f.module, l.slot);
                            listed.iter().find(|i| i.path == path && i.kind == "trait")
                        });
                    match (code(&l.entry), code(&generic), slot) {
                        (Some(entry), Some(generic_fn), Some(slot)) => {
                            let publication =
                                format!("{}|{}|{}|{}", l.entry, l.generic, l.slot, l.side);
                            Now::fingerprint(
                                format!(
                                    "{publication}|{}|{}|{}",
                                    span_hash(entry),
                                    span_hash(generic_fn),
                                    slot.hash
                                ),
                                Some(format!("{publication}|{}", slot.hash)),
                                [(l.entry.as_str(), entry), (generic.as_str(), generic_fn)],
                            )
                        }
                        _ => Now::Unjudged,
                    }
                }
            };
            entries.push((format!("{PREFIX}loop:{}", row.name), now));
        }
        for declared in &manifest.internals {
            let found = internals::resolve(
                listed,
                &manifest.id,
                &declared.path,
                declared.kind.as_deref(),
            );
            // The id is what the mod declares, so a record is found again whatever the item became;
            // the kind it resolved to is part of the fingerprint.
            let id = format!(
                "{PREFIX}internal:{}:{}",
                declared.kind.as_deref().unwrap_or("any"),
                declared.path
            );
            let now = match found {
                Err(_) => Now::Gone,
                Ok(internal) => match internal.kind.as_str() {
                    "fn" | "method" => code(&internal.path).map_or(Now::Unjudged, |f| {
                        // The function's declaration digest already covers the signature and
                        // impl context that the internals table hashes separately.
                        Now::fingerprint(
                            format!("{}:{}:{}", internal.kind, internal.hash, span_hash(f)),
                            None,
                            [(internal.path.as_str(), f)],
                        )
                    }),
                    kind => Now::Is {
                        hash: format!("{kind}:{}", internal.hash),
                        game_code: None,
                    },
                },
            };
            entries.push((id, now));
        }
        entries.sort();
        entries.dedup();
        entries
    }
}

/// How a build's [`entries`] stand against a list's record of them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Compared {
    /// What is gone or not as recorded, in id order.
    pub misfits: Vec<Misfit>,
    /// Whether the list has no record of something the mod declares that the build can fingerprint: packing the mod again writes it.
    pub unrecorded: bool,
    /// The ids the build cannot fingerprint ([`Now::Unjudged`]).
    pub unjudged: Vec<String>,
}

/// `now`, a build's [`entries`], against the list's `recorded` entries.
pub fn compare(recorded: &[&Entry], now: Vec<(String, Now)>) -> Compared {
    let mut compared = Compared::default();
    for (id, now) in now {
        let made = recorded.iter().find(|entry| entry.id == id);
        let change = match (made, now) {
            (_, Now::Gone) => Change::Removed,
            (_, Now::Refused { code, why }) => Change::Refused { code, why },
            (_, Now::Unjudged) => {
                compared.unjudged.push(id);
                continue;
            }
            (None, Now::Is { .. }) => {
                compared.unrecorded = true;
                continue;
            }
            (Some(made), Now::Is { hash, game_code }) => {
                match (&made.game_code, game_code) {
                    (Some(old), Some(new)) => {
                        // The raw span still verifies source bytes. It is not fit evidence when
                        // both builds can compare tokens, and it misses edits to an impl header.
                        for (function, now) in &new.functions {
                            if let Some(made) = old.functions.get(function) {
                                let change = if made.declaration != now.declaration {
                                    Change::DeclarationChanged {
                                        function: function.clone(),
                                    }
                                } else if made.body != now.body {
                                    Change::BodyChanged {
                                        function: function.clone(),
                                    }
                                } else {
                                    continue;
                                };
                                compared.misfits.push(Misfit {
                                    id: id.clone(),
                                    change,
                                });
                            }
                        }
                        if old.loop_contract == new.loop_contract
                            && old.functions.keys().eq(new.functions.keys())
                        {
                            continue;
                        }
                        Change::Changed
                    }
                    _ if made.hash != hash => Change::Changed,
                    _ => continue,
                }
            }
        };
        compared.misfits.push(Misfit { id, change });
    }
    compared
}
