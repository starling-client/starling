//! What Horizon Android's launcher drew with from its game (`game::loading_renderer`,
//! `UiResources`, `loading_frame`), over Starling's boot renderer (`gfx::boot`): the launcher draws
//! a HUD batch and nothing else, and the boot renderer opens the device with only the two
//! pipelines that takes, where the world's renderer builds every pipeline the game will need (on a
//! PowerVR GE8300, about 40 seconds of them).
use crate::ha_launcher::client::{
    Result,
    gfx::{boot::BootRenderer, panorama::Faces},
    graphics::GraphicsApi,
    hud::{HudAtlas, HudBatch},
    sound_events::SoundRegistry,
};
use std::sync::Arc;
use winit::window::Window;

/// The surface's size, where Horizon Android read the world renderer's configuration.
#[derive(Clone, Copy)]
pub struct Surface {
    pub width: u32,
    pub height: u32,
}

/// The launcher's renderer.
pub struct Renderer {
    boot: BootRenderer,
    window: Arc<Window>,
    /// The window is in the background: no frame draws until it comes back.
    pub suspended: bool,
    pub config: Surface,
    pub info: wgpu::AdapterInfo,
    /// The first frame reached the screen: StarlingActivity holds its window back until then.
    shown: bool,
}

impl Renderer {
    pub fn has_presented(&self) -> bool {
        self.shown && !self.suspended && !self.boot.detached
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.boot.resize(width, height);
        let [width, height] = self.boot.size();
        self.config = Surface { width, height };
    }
    /// The window came back: a new surface on it.
    pub fn recreate_surface(&mut self) -> Result<()> {
        let size = self.window.inner_size();
        self.boot.resume(size.width, size.height)?;
        let [width, height] = self.boot.size();
        self.config = Surface { width, height };
        self.suspended = false;
        Ok(())
    }
    pub fn suspend_surface(&mut self) {
        self.boot.suspend();
        self.suspended = true;
    }
    /// The GUI atlas the launcher draws with, once it is built.
    pub fn set_loading_ui(&mut self, atlas: &HudAtlas, panorama: &Faces) -> Result<()> {
        atlas.font.reset_cache();
        let _ = panorama;
        self.boot.set_ui(
            (atlas.pixels.as_slice(), atlas.width, atlas.height),
            Arc::new(Faces::colour([0, 0, 0, 255])),
        )
    }
    /// One frame; false when none was drawn (the window is in the background).
    pub fn render(&mut self, batch: &HudBatch) -> Result<bool> {
        if self.suspended || self.boot.detached {
            return Ok(false);
        }
        if !self.boot.render(batch, 0.0, false)? {
            return Ok(false);
        }
        if !self.shown {
            self.shown = true;
            crate::android::first_frame_shown();
        }
        Ok(true)
    }
}

/// The launcher's GUI resources: the atlas (its font and sprites), a title panorama it does not
/// show, and the GUI's sounds.
pub struct UiResources {
    pub atlas: HudAtlas,
    pub panorama: Arc<Faces>,
    pub sounds: Option<SoundRegistry>,
}

/// The renderer on the launcher's window.
pub fn loading_renderer(
    window: Arc<Window>,
    _offscreen: bool,
    _atlas: Option<&HudAtlas>,
    _panorama: Option<&Arc<Faces>>,
    _previous: Option<&Renderer>,
    api: GraphicsApi,
) -> Result<Renderer> {
    static EMPTY_HUD: [u8; 4] = [0, 0, 0, 255];
    let boot = pollster::block_on(BootRenderer::new(
        window.clone(),
        (EMPTY_HUD.as_slice(), 1, 1),
        Arc::new(Faces::colour([0, 0, 0, 255])),
        api,
    ))?;
    let [width, height] = boot.size();
    let info = boot.info();
    Ok(Renderer {
        boot,
        window,
        suspended: false,
        config: Surface { width, height },
        info,
        shown: false,
    })
}

/// The frame of a batch: the boot renderer draws the batch itself.
pub fn loading_frame(batch: &HudBatch) -> &HudBatch {
    batch
}

/// Horizon Android's game kept the mods it bundled on between runs; Starling's has none.
pub fn android_reset_bundled_mods() {}
