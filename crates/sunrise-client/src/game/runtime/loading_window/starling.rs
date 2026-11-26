//! Starling's part of loading_window.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// A driver that dies while the first renderer starts leaves this marker behind, and the next
/// start opens with OpenGL ES (`graphics::RENDERER_MARKER`). A renderer made on a device that
/// has shown frames already (`previous`) writes none.
#[cfg(target_os = "android")]
pub(super) fn start_marker(previous: Option<&Renderer>) -> Option<std::path::PathBuf> {
    previous.is_none().then(|| {
        let marker = Game::options_path().with_file_name(sunrise_client::graphics::RENDERER_MARKER);
        let _ = std::fs::write(&marker, "the renderer was starting\n");
        sunrise_client::graphics::note_start_marker(&marker);
        marker
    })
}

/// What the world's renderer is made from (`Renderer::new`): borrowed from the pack and the GUI,
/// or owned (`into_owned`) so that it is built on a thread of its own while the loading screen
/// goes on drawing (`Loading::begin_world_renderer`). The GUI's font stays on the main thread,
/// where `finish_renderer` does the rest.
pub(in super::super) struct RendererParts<'a> {
    pub(super) window: Arc<Window>,
    pub(super) offscreen: bool,
    pub(super) pack: Cow<'a, PackResources>,
    // The GUI atlas's RGBA (`HudAtlas::pixels`), its width and its height.
    pub(super) atlas: (Cow<'a, [u8]>, u32, u32),
    pub(super) glint: (Cow<'a, [u8]>, u32, u32),
    pub(super) glint_armor: (Cow<'a, [u8]>, u32, u32),
    pub(super) panorama: Arc<sunrise_client::gfx::panorama::Faces>,
    pub(super) prepared: Option<sunrise_client::gfx::PreparedTextures>,
    pub(super) api: sunrise_client::graphics::GraphicsApi,
    pub(super) translucent_batching: bool,
    pub(super) settings: Cow<'a, sunrise_client::graphics::Graphics>,
    pub(super) shader_pack: shader_packs::PickedPack,
    pub(super) display: sunrise_client::gfx::DisplayReading,
}

pub(super) fn borrowed(image: &(Vec<u8>, u32, u32)) -> (Cow<'_, [u8]>, u32, u32) {
    (Cow::Borrowed(image.0.as_slice()), image.1, image.2)
}

pub(super) fn owned(image: (Cow<'_, [u8]>, u32, u32)) -> (Cow<'static, [u8]>, u32, u32) {
    (Cow::Owned(image.0.into_owned()), image.1, image.2)
}

impl<'a> RendererParts<'a> {
    pub(in super::super) fn new(
        window: Arc<Window>,
        offscreen: bool,
        pack: &'a PackResources,
        gui: &'a UiResources,
        // `gui`'s atlas pixels: `HudAtlas::pixels`, or the ones taken from it for a thread.
        atlas_pixels: Cow<'a, [u8]>,
        prepared: Option<sunrise_client::gfx::PreparedTextures>,
        api: sunrise_client::graphics::GraphicsApi,
        translucent_batching: bool,
        settings: &'a sunrise_client::graphics::Graphics,
        shader_pack: shader_packs::PickedPack,
        display: sunrise_client::gfx::DisplayReading,
    ) -> Self {
        let atlas = &gui.atlas;
        Self {
            window,
            offscreen,
            pack: Cow::Borrowed(pack),
            atlas: (atlas_pixels, atlas.width, atlas.height),
            glint: borrowed(&atlas.glint),
            glint_armor: borrowed(&atlas.glint_armor),
            panorama: gui.panorama.clone(),
            prepared,
            api,
            translucent_batching,
            settings: Cow::Borrowed(settings),
            shader_pack,
            display,
        }
    }

    /// The same parts, owning what they borrowed: the pack is shared (`PackResources` clones its
    /// handles), the glint textures and the settings copied, and atlas pixels given owned stay
    /// as they are.
    pub(in super::super) fn into_owned(self) -> RendererParts<'static> {
        RendererParts {
            window: self.window,
            offscreen: self.offscreen,
            pack: Cow::Owned(self.pack.into_owned()),
            atlas: owned(self.atlas),
            glint: owned(self.glint),
            glint_armor: owned(self.glint_armor),
            panorama: self.panorama,
            prepared: self.prepared,
            api: self.api,
            translucent_batching: self.translucent_batching,
            settings: Cow::Owned(self.settings.into_owned()),
            shader_pack: self.shader_pack,
            display: self.display,
        }
    }

    /// `Renderer::new` from these parts, on whichever thread holds them. `previous` is the
    /// renderer this replaces on the same device; `boot` the loading screen's device
    /// (`BootRenderer::lend`). The atlas pixels come back with it.
    pub(in super::super) fn build(
        self,
        previous: Option<&Renderer>,
        boot: Option<sunrise_client::gfx::boot::Lent>,
    ) -> Result<(Renderer, Cow<'a, [u8]>)> {
        let assets = &self.pack.render;
        // Made here when the worker did not: the renderer's own fallback has no LabPBR maps.
        let prepared = Some(
            self.prepared
                .unwrap_or_else(|| prepare_renderer_textures(&self.pack)),
        );
        #[cfg(target_os = "android")]
        crate::diagnostics::checkpoint(
            "renderer_init",
            || serde_json::json!({"api":format!("{:?}",self.api)}),
        );
        #[cfg(target_os = "android")]
        let marker = start_marker(previous);
        let (atlas_pixels, atlas_width, atlas_height) = self.atlas;
        let renderer = pollster::block_on(Renderer::new(
            self.window,
            self.offscreen,
            &assets.pixels.get(),
            assets.sprite_size,
            assets.sprite_minimum_size,
            assets.texture_layers,
            &assets.sprite_mips,
            &assets.animations,
            (&atlas_pixels, atlas_width, atlas_height),
            (&self.glint.0, self.glint.1, self.glint.2),
            (&self.glint_armor.0, self.glint_armor.1, self.glint_armor.2),
            (&self.pack.forcefield.2, self.pack.forcefield.0),
            self.panorama,
            prepared,
            previous,
            boot,
            self.api,
            self.translucent_batching,
            (self.shader_pack.draws_flat, self.shader_pack.starts_wide),
            &self.settings,
            self.shader_pack.frame(),
            self.display,
        ))?;
        #[cfg(target_os = "android")]
        let renderer = {
            let mut renderer = renderer;
            if let Some(marker) = marker {
                renderer.forget_start_marker(marker);
            }
            renderer
        };
        // The pipelines it compiled, for the next start (Android's Vulkan).
        renderer.save_pipeline_cache();
        Ok((renderer, atlas_pixels))
    }
}

/// The loading screen's own renderer, in place of `loading_renderer` wherever there is a window
/// to draw on: the same atlas and panorama, and none of the world's pipelines.
pub(in super::super) fn boot_renderer(
    window: Arc<Window>,
    atlas: Option<&sunrise_client::hud::HudAtlas>,
    panorama: Option<&Arc<sunrise_client::gfx::panorama::Faces>>,
    api: sunrise_client::graphics::GraphicsApi,
) -> Result<sunrise_client::gfx::boot::BootRenderer> {
    static EMPTY_HUD: [u8; 4] = [0, 0, 0, 255];
    let faces = panorama
        .cloned()
        .unwrap_or_else(|| Arc::new(sunrise_client::gfx::panorama::Faces::colour([0; 4])));
    let (pixels, width, height) = atlas.map_or((EMPTY_HUD.as_slice(), 1, 1), |a| {
        (a.pixels.as_slice(), a.width, a.height)
    });
    // As `loading_renderer`: a glyph rasterized into an earlier texture is not in these pixels.
    if let Some(atlas) = atlas {
        atlas.font.reset_cache();
    }
    #[cfg(target_os = "android")]
    crate::diagnostics::checkpoint(
        "renderer_init",
        || serde_json::json!({"api":format!("{:?}",api)}),
    );
    // As `loading_renderer`: the marker that sends the next start to OpenGL ES if this one dies
    // opening the device or in its first frame, removed with that frame.
    #[cfg(target_os = "android")]
    let marker = start_marker(None);
    let boot = pollster::block_on(sunrise_client::gfx::boot::BootRenderer::new(
        window,
        (pixels, width, height),
        faces,
        api,
    ))?;
    #[cfg(target_os = "android")]
    let boot = {
        let mut boot = boot;
        if let Some(marker) = marker {
            boot.forget_start_marker(marker);
        }
        boot
    };
    Ok(boot)
}

/// The menu bundle's atlas and title panorama replacing the boot screen's, on the device the
/// boot renderer already opened.
pub(in super::super) fn boot_set_ui(
    boot: &mut sunrise_client::gfx::boot::BootRenderer,
    atlas: &sunrise_client::hud::HudAtlas,
    panorama: &Arc<sunrise_client::gfx::panorama::Faces>,
) -> Result<()> {
    atlas.font.reset_cache();
    boot.set_ui(
        (atlas.pixels.as_slice(), atlas.width, atlas.height),
        panorama.clone(),
    )
}
