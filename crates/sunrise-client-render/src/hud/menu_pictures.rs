//! The menu pictures the GUI atlas leaves out: the title panorama and the world-preset
//! pictures, drawn by the menu on their own.

/// A texture (`assets/<ns>/textures/...`) the menu draws on its own, not from the GUI
/// atlas: the title panorama and the world-preset pictures. Realms slot thumbnails also
/// use the menu picture cache. Only those two folders count, so a sprite whose name holds `presets`
/// (a registered atlas's `gui/atlases/presets/...`) is still stitched.
pub(super) fn menu_picture_only(path: &str) -> bool {
    let Some((_, texture)) = path
        .strip_prefix("assets/")
        .and_then(|rest| rest.split_once('/'))
    else {
        return false;
    };
    texture.starts_with("textures/gui/title/background/")
        || texture.starts_with("textures/gui/presets/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_menu_picture_folders_stay_out_of_the_gui_atlas() {
        for kept_out in [
            "assets/minecraft/textures/gui/title/background/panorama_0.png",
            "assets/minecraft/textures/gui/title/background/panorama_1.png",
            "assets/minecraft/textures/gui/title/background/panorama_2.png",
            "assets/minecraft/textures/gui/title/background/panorama_3.png",
            "assets/minecraft/textures/gui/title/background/panorama_4.png",
            "assets/minecraft/textures/gui/title/background/panorama_5.png",
            "assets/minecraft/textures/gui/presets/isles.png",
            "assets/minecraft/textures/gui/presets/desert.png",
        ] {
            assert!(menu_picture_only(kept_out), "{kept_out}");
        }
        for stitched in [
            "assets/mymod/textures/gui/atlases/presets/mymod/heart.png",
            "assets/mymod/textures/gui/atlases/icons/mymod/title/background/sky.png",
            "assets/minecraft/textures/gui/sprites/icon/presets.png",
        ] {
            assert!(!menu_picture_only(stitched), "{stitched}");
        }
    }
}
