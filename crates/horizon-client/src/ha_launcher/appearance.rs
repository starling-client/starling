//! The wardrobe's choice, worn by Starling's game: Horizon Android's game read the launcher's
//! `horizon-appearance.json`; Starling's wears the choice each profile keeps in the instance
//! (`own_skin`: `horizon-skins.json` and the PNGs of `skins/`), which its Skin Customization
//! shows too.
use std::path::Path;

/// Keep `skin` (a picture of the wardrobe's library, or None for the profile's own) as the choice
/// of the profile that plays, in the instance `game`.
pub fn wear(game: &Path, skin: Option<&Path>, slim: bool) -> Result<(), String> {
    use crate::ha_launcher::client::own_skin::{DIR, SkinChoice};
    let file = match skin {
        Some(path) => {
            let id = path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("The wardrobe's skin has no name.")?;
            let name = format!("wardrobe-{id}");
            let dir = game.join(DIR);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::copy(path, dir.join(&name)).map_err(|e| e.to_string())?;
            Some(name)
        }
        None => None,
    };
    let key = starling_account::Accounts::load(&crate::accounts::dir())
        .player()
        .map(|p| p.key());
    SkinChoice { file, slim }
        .save_for_profile(game, key.as_deref())
        .map_err(|e| e.to_string())
}

/// Horizon Android's offline name plays as Starling's offline profile of that name.
pub fn play_offline(name: &str) {
    let name = name.trim();
    if starling_account::valid_offline_name(name)
        && let Err(error) = starling_account::add_offline(&crate::accounts::dir(), name)
    {
        log::warn!("Launcher: offline profile {name}: {error}");
    }
}
