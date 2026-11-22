//! Resolve the staged graph on the game's registry pins without downloading archives.
use super::registry_lock::{Identity, RegistryLock};
use crate::host::trf;
use std::{collections::BTreeMap, fs, path::Path, process::Command};
use sunrise_mod_package::native_route::game_registry_pin_conflicts;

/// Start from the supplied seed, and constrain only game pins Cargo actually tries to move.
/// The temporary root gives Cargo exact requirements, so it can backtrack to another parent
/// instead of concluding that the preferred parent is the only possible graph.
pub(super) fn resolve(
    directory: &Path,
    game: &RegistryLock,
    seed: &RegistryLock,
    allowed: Option<&RegistryLock>,
    mut cargo: impl FnMut(&str) -> Command,
) -> Result<String, String> {
    let lock_path = directory.join("Cargo.lock");
    const EMPTY_LOCK: &str = "version=4\npackage=[]\n";
    let original_seed = seed.seed(EMPTY_LOCK)?;
    let mut update = || {
        let mut command = cargo("update");
        // `generate-lockfile` would ignore the seed.
        command.args(["--workspace", "--color", "never"]);
        super::run(command, "Resolving registry dependency pins", |_| {})?;
        let text = super::read_lock(directory)?;
        let resolved = RegistryLock::parse(&text)?;
        Ok::<_, String>((text, resolved))
    };
    fs::write(&lock_path, original_seed).map_err(|e| e.to_string())?;
    let (text, resolved) = update()?;
    if game.moved_pins(&resolved).next().is_none() {
        if let Some(allowed) = allowed {
            allowed.check_pins(&resolved)?;
        }
        return Ok(text);
    }

    // Cargo may turn the first of several mod pins into a locked requirement. On the
    // constrained pass, let it try all versions for those names; the build still checks that
    // each selected version and checksum was supplied by a lock before fetching anything.
    let mut alternatives = BTreeMap::<_, Vec<_>>::new();
    for pin in seed
        .packages
        .keys()
        .filter(|id| !game.packages.contains_key(*id))
    {
        alternatives.entry(pin.name.as_str()).or_default().push(pin);
    }
    let mut retry_seed = seed.clone();
    retry_seed.packages.retain(|id, _| {
        game.packages.contains_key(id)
            || alternatives
                .get(id.name.as_str())
                .is_none_or(|pins| pins.len() < 2)
    });
    let unlocked = retry_seed.seed(EMPTY_LOCK)?;

    let manifest_path = directory.join("Cargo.toml");
    let original = fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
    let mut manifest: toml::Table = original.parse().map_err(|e| format!("Cargo.toml: {e}"))?;
    let pin_root = tempfile::Builder::new()
        .prefix("sunrise-game-pins-")
        .tempdir_in(directory)
        .map_err(|e| e.to_string())?;
    let name = pin_root
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Invalid temporary registry pin directory")?;
    let workspace = manifest
        .get_mut("workspace")
        .and_then(toml::Value::as_table_mut)
        .ok_or("Missing staged Cargo workspace")?;
    workspace
        .entry("members")
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("Invalid staged Cargo workspace members")?
        .push(name.into());
    fs::write(pin_root.path().join("inert.rs"), "").map_err(|e| e.to_string())?;
    let mut pins = toml::toml! { [lib] path = "inert.rs" };
    pins.insert(
        "package".into(),
        toml::Table::from_iter([
            ("name".into(), name.into()),
            ("version".into(), "0.0.0".into()),
            ("edition".into(), "2024".into()),
        ])
        .into(),
    );
    fs::write(&manifest_path, manifest.to_string()).map_err(|e| e.to_string())?;

    let mut resolve_with_constraints = |retry_seed: &str| {
        let mut constraints = BTreeMap::new();
        let mut resolved = resolved.clone();
        loop {
            // A different parent may expose another moved line. Every pass adds at least one
            // game identity or exits, so this is bounded by the number of pins in the game lock.
            let mut added = false;
            for (game, selected) in game.moved_pins(&resolved) {
                added |= constraints.insert(game, selected.clone()).is_none();
            }
            if !added {
                break;
            }
            let dependencies: toml::Table = constraints
                .keys()
                .enumerate()
                .map(|(index, pin)| {
                    let spec = toml::Table::from_iter([
                        ("package".into(), pin.name.clone().into()),
                        ("version".into(), format!("={}", pin.version).into()),
                        ("default-features".into(), false.into()),
                    ]);
                    (format!("pin_{index}"), spec.into())
                })
                .collect();
            pins.insert("dependencies".into(), dependencies.into());
            fs::write(pin_root.path().join("Cargo.toml"), pins.to_string())
                .map_err(|e| e.to_string())?;
            fs::write(&lock_path, retry_seed).map_err(|e| e.to_string())?;
            resolved = update()
                .map_err(|error| {
                    format!(
                        "{}\n\n{error}",
                        pin_summary(constraints.iter().map(|(&a, b)| (a, b)))
                    )
                })?
                .1;
        }
        Ok::<_, String>(resolved)
    };
    let constrained = (|| {
        let resolved = resolve_with_constraints(&unlocked)?;
        let Some(allowed) = allowed else {
            return Ok(());
        };
        let Err(unpinned) = allowed.check_pins(&resolved) else {
            return Ok(());
        };

        // Unlocking a package also exposes newly published versions that are not authorized.
        // Only if that happens, try the supplied preferences one per compatible line at a time.
        // Keep the distinct lines together: Cargo may use several of them in the same graph.
        let mut lines: Vec<Vec<&Identity>> = Vec::new();
        for pins in alternatives.values().filter(|pins| pins.len() > 1) {
            for &pin in pins {
                if let Some(line) = lines.last_mut().filter(|line| {
                    line[0].name == pin.name
                        && game_registry_pin_conflicts(&line[0].version, &pin.version)
                }) {
                    line.push(pin);
                } else {
                    lines.push(vec![pin]);
                }
            }
        }
        if lines.is_empty() {
            return Err(unpinned);
        }
        let mut choices: Vec<_> = lines.iter().map(|pins| pins.len() - 1).collect();
        loop {
            crate::check_cancelled()?;
            let mut attempt = retry_seed.clone();
            for (pins, &choice) in lines.iter().zip(&choices) {
                let pin = pins[choice];
                attempt
                    .packages
                    .insert(pin.clone(), seed.packages[pin].clone());
            }
            if let Ok(resolved) = resolve_with_constraints(&attempt.seed(EMPTY_LOCK)?)
                && allowed.check_pins(&resolved).is_ok()
            {
                return Ok(());
            }
            let Some(next) = choices.iter().rposition(|&choice| choice != 0) else {
                return Err(unpinned);
            };
            choices[next] -= 1;
            for (choice, pins) in choices[next + 1..].iter_mut().zip(&lines[next + 1..]) {
                *choice = pins.len() - 1;
            }
        }
    })();
    fs::write(&manifest_path, original).map_err(|e| e.to_string())?;
    drop(pin_root);
    constrained?;

    // With the chosen graph now in Cargo.lock, prune the temporary root and dependencies
    // reachable only from it. They must not be downloaded or compiled for the mod.
    let (text, resolved) = update()?;
    check_game_pins(game, &resolved)?;
    if let Some(allowed) = allowed {
        allowed.check_pins(&resolved)?;
    }
    Ok(text)
}

pub(super) fn check_game_pins(game: &RegistryLock, resolved: &RegistryLock) -> Result<(), String> {
    let mut moved = game.moved_pins(resolved).peekable();
    if moved.peek().is_some() {
        Err(pin_summary(moved))
    } else {
        Ok(())
    }
}

fn pin_summary<'a>(pins: impl Iterator<Item = (&'a Identity, &'a Identity)>) -> String {
    pins.map(|(game, selected)| {
        trf(
            "{crate}: the game locks {game}, but resolution without the game's constraint selected {selected}.",
            &[
                ("crate", &game.name),
                ("game", &game.version.to_string()),
                ("selected", &selected.version.to_string()),
            ],
        )
    })
    .collect::<Vec<_>>()
    .join("\n")
}
