//! Whether a native mod fits the Horizon the player plays, said before it is installed, so a
//! mod made for another Horizon is no surprise at build time. Diverges from upstream: native
//! mods pin an exact commit, with no hook-version compatibility or automatic revision change.
use crate::i18n::{tr, trf};
use crate::instances::Instance;
use crate::{backend, native_mods};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The Horizon the selected instance plays: the commit its game was compiled from.
#[derive(Clone, Debug, PartialEq)]
pub struct Plays {
    pub revision: String,
}

/// How a native mod fits [`Plays`].
#[derive(Clone, Debug, PartialEq)]
pub enum Fit {
    /// It builds on the Horizon the player plays.
    Fits,
    /// It pins another commit, and mods turned on pin yet another one. The build refuses
    /// mismatched pins rather than changing them.
    Clashes { pin: String, with: Vec<String> },
    /// It pins another commit: the player must choose matching mods or a matching game.
    OtherRevision { pin: String },
}

/// How mod `id`, which pins commit `pin`, fits the Horizon played. `pins` are the mods turned
/// on that pin a commit (id, commit); the mod's own older version is replaced by this one.
pub fn fit(id: &str, pin: &str, plays: &Plays, pins: &[(String, String)]) -> Fit {
    if pin == plays.revision {
        return Fit::Fits;
    }
    let with: Vec<String> = pins
        .iter()
        .filter(|(other, revision)| other != id && revision != pin)
        .map(|(other, _)| other.clone())
        .collect();
    match with.is_empty() {
        true => Fit::OtherRevision { pin: pin.into() },
        false => Fit::Clashes {
            pin: pin.into(),
            with,
        },
    }
}

fn short(revision: &str) -> &str {
    &revision[..revision.len().min(7)]
}

/// What the player reads about `fit`.
pub fn describe(fit: &Fit, plays: &Plays) -> String {
    match fit {
        Fit::Fits => tr("Builds on the Horizon you play").to_owned(),
        Fit::Clashes { pin, with } => trf(
            "Made for Horizon {pin}, but {mods} pin another commit: the build requires all native mods to match the game revision",
            &[("pin", short(pin)), ("mods", &with.join(", "))],
        ),
        Fit::OtherRevision { pin } => trf(
            "Made for Horizon {pin}, you play {yours}: choose matching mod versions or a matching Horizon version before building",
            &[("pin", short(pin)), ("yours", short(&plays.revision))],
        ),
    }
}

/// [`Plays`] by the version an instance plays: a known commit is kept, a failure for [`RETRY`].
/// Only resolving a release or Nightly commit asks GitHub, once for all the search's results.
type CacheKey = (String, Option<String>);
type Known = HashMap<CacheKey, (Instant, Option<Plays>)>;
static PLAYS: Mutex<Option<Known>> = Mutex::new(None);
const RETRY: Duration = Duration::from_secs(60);

fn cached(cache: &Known, key: &CacheKey) -> Option<Option<Plays>> {
    cache
        .get(key)
        .filter(|(at, plays)| plays.is_some() || at.elapsed() < RETRY)
        .map(|(_, plays)| plays.clone())
}

/// The Horizon `instance` plays; None for a Java instance, when nothing is installed, or when
/// GitHub can't say its commit.
pub fn plays(instance: &Instance) -> Option<Plays> {
    if instance.java.is_some() {
        return None;
    }
    let version = instance.resolve_version(None).ok()?;
    // Read the build record before consulting the cache so a rebuild's answer follows it.
    let native_revision = native_mods::about(&version).map(|about| about.horizon);
    let key = (version.clone(), native_revision.clone());
    let known = cached(PLAYS.lock().unwrap().get_or_insert_default(), &key);
    if let Some(plays) = known {
        return plays;
    }
    // A native build is the commit it compiled; anything else, the commit a build would use.
    let revision = match native_revision {
        Some(revision) => Some(revision),
        None => native_mods::base_for(Some(&version), &backend::installed(), false)
            .and_then(|base| native_mods::revision_of(&base).ok()),
    };
    let plays = revision.map(|revision| Plays { revision });
    PLAYS
        .lock()
        .unwrap()
        .get_or_insert_default()
        .insert(key, (Instant::now(), plays.clone()));
    plays
}

/// The native mods turned on in game folder `home` that pin a commit, as (id, commit).
pub fn pins(home: &Path) -> Vec<(String, String)> {
    crate::native_status::bundles_on(home)
        .into_iter()
        .map(|b| {
            let native = b.manifest.native.expect("native bundle");
            (b.manifest.id, native.horizon)
        })
        .collect()
}

/// The line the registry browser shows for a native mod's version pinning `pin`, in `instance`;
/// None when the game's revision can't be told. This only describes fit: it never builds or
/// changes the instance or a package pin.
pub fn line(instance: &Instance, id: &str, pin: &str) -> Option<String> {
    let plays = plays(instance)?;
    Some(describe(
        &fit(id, pin, &plays, &pins(&instance.home)),
        &plays,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const YOURS: &str = "1111111111111111111111111111111111111111";
    const OTHER: &str = "2222222222222222222222222222222222222222";
    const THIRD: &str = "3333333333333333333333333333333333333333";

    fn plays() -> Plays {
        Plays {
            revision: YOURS.into(),
        }
    }

    fn pin(id: &str, revision: &str) -> (String, String) {
        (id.into(), revision.into())
    }

    #[test]
    fn rebuilding_a_native_version_does_not_reuse_its_previous_compatibility() {
        let version = "native-example".to_string();
        let old_key = (version.clone(), Some(YOURS.to_string()));
        let rebuilt_key = (version.clone(), Some(OTHER.to_string()));
        let unbuilt_key = (version, None);
        let old = plays();
        let mut cache = Known::new();
        cache.insert(old_key.clone(), (Instant::now(), Some(old.clone())));
        assert_eq!(cached(&cache, &old_key), Some(Some(old)));
        assert_eq!(cached(&cache, &rebuilt_key), None);
        assert_eq!(cached(&cache, &unbuilt_key), None);
    }

    #[test]
    fn a_pinned_mod_fits_only_its_commit() {
        let p = plays();
        assert_eq!(fit("jei", YOURS, &p, &[]), Fit::Fits);
        assert_eq!(
            fit("jei", OTHER, &p, &[]),
            Fit::OtherRevision { pin: OTHER.into() }
        );
    }

    #[test]
    fn mods_pinning_another_commit_clash() {
        let pins = [
            pin("veinminer", THIRD),
            pin("chop", OTHER),
            pin("jei", THIRD),
        ];
        // `chop` agrees with the pin and `jei` is the mod being replaced: only veinminer clashes.
        assert_eq!(
            fit("jei", OTHER, &plays(), &pins),
            Fit::Clashes {
                pin: OTHER.into(),
                with: vec!["veinminer".into()],
            }
        );
        let agreeing = [pin("chop", OTHER)];
        assert_eq!(
            fit("jei", OTHER, &plays(), &agreeing),
            Fit::OtherRevision { pin: OTHER.into() }
        );
    }
}
