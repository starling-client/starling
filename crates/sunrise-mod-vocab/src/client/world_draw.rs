//! Untextured, unlit geometry submitted for one world view.

/// Whether world geometry is hidden by nearer world surfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local depth policy for world geometry.
pub enum WorldDepth {
    /// Test against the world's depth, without writing depth.
    Tested,
    /// Draw over world surfaces, without testing or writing depth.
    ThroughWalls,
}

/// A world-space line, colored with 0xAARRGGBB and measured in screen pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local world line submission.
pub struct WorldLine {
    /// Starting position in blocks, before camera subtraction.
    pub from: [f64; 3],
    /// Ending position in blocks, before camera subtraction.
    pub to: [f64; 3],
    /// Packed ARGB color, including alpha.
    pub color: u32,
    /// Finite, positive width in pixels.
    pub width: f32,
    /// Occlusion behavior.
    pub depth: WorldDepth,
}
impl WorldLine {
    /// Describe a line; the draw context validates it when submitting the batch.
    pub fn new(from: [f64; 3], to: [f64; 3], color: u32, width: f32, depth: WorldDepth) -> Self {
        Self {
            from,
            to,
            color,
            width,
            depth,
        }
    }
}

/// A two-sided, unlit quad. Corners run top left, bottom left, bottom right,
/// top right as seen from the front; alpha blends without writing depth.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
// no wire: local world quad submission.
pub struct WorldQuad {
    /// Four world-space corners in blocks, before camera subtraction.
    pub corners: [[f64; 3]; 4],
    /// Packed 0xAARRGGBB color, including alpha.
    pub color: u32,
    /// Occlusion behavior.
    pub depth: WorldDepth,
}
impl WorldQuad {
    /// Describe a quad; the draw context validates it when submitting the batch.
    pub fn new(corners: [[f64; 3]; 4], color: u32, depth: WorldDepth) -> Self {
        Self {
            corners,
            color,
            depth,
        }
    }
}

/// Why a whole geometry batch or label was refused; nothing from that call was appended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: refusal of an in-process draw submission.
pub enum DrawError {
    /// A coordinate or width is not finite, or a camera-relative coordinate overflows f32.
    NonFinite,
    /// A finite line width is zero or negative.
    InvalidWidth,
    /// The selected stage has no render pass.
    NoRenderPass,
}

// no wire: a draw handle only names a batch in the current in-process render view.
// exhaustive: its fields are private; only the renderer constructs a draw handle
/// Geometry submitted in `prepare_render_buffers`. Use it in the matching stage of this view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedWorldDraw {
    view: u64,
    slot: usize,
}

impl PreparedWorldDraw {
    // hidden: the renderer owns the view and its batch table.
    #[doc(hidden)]
    pub fn __new(view: u64, slot: usize) -> Self {
        Self { view, slot }
    }

    // hidden: the renderer checks the view before resolving the slot.
    #[doc(hidden)]
    pub fn __slot(self, view: u64) -> Option<usize> {
        (self.view == view).then_some(self.slot)
    }
}

// no wire: only returned when resolving an in-process draw handle.
// exhaustive: unit error marker carrying no fields
/// A draw handle was not prepared for this view and stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedWorldDrawError;

/// How [`WorldDrawContext::name_tag`](crate::WorldDrawContext::name_tag) draws a label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local world label drawing options.
pub struct NameTagOptions {
    /// Draw it as vanilla draws a name that is not sneaking: opaque text in the world and a
    /// faded copy over everything, walls included. Off, it is drawn once as a sneaking name:
    /// the faded text, depth tested and translucent. The faded text's alpha follows the
    /// text background opacity unless `faded_alpha` is set: 255 times the larger of
    /// (opacity + 0.75) / 2 and 0.5, 0x7F at the default.
    pub see_through: bool,
    /// Put the translucent black backdrop behind the faded text, at the player's text
    /// background opacity (`Options.getBackgroundOpacity(0.25)`: 0x3F at the default).
    pub background: bool,
    /// The light at the label, as a name tag reads its entity's: sky light in the high four
    /// bits, block light in the low four (`0xF0` is full sky light and no block light; `0x0F`
    /// full block light under no sky). Without `see_through` the label is drawn in exactly
    /// this light; with it, the copy in the world raises each of the two levels to at least 2
    /// (`lightCoordsWithEmission(light, 2)`) and the copy over everything ignores it.
    pub light: u8,
    /// The alpha byte of the faded text, whichever copy it is: the see-through copy, or the
    /// single copy when `see_through` is off (`Font.drawInBatch` takes the colour's alpha from
    /// its caller). `None` keeps the value derived from the text background opacity above.
    /// Fragments under alpha 0.1 are discarded as for every name tag copy, so an alpha under 26
    /// draws no text. The backdrop is unchanged.
    pub faded_alpha: Option<u8>,
    /// With `see_through`, draw the opaque copy in the world in `light` exactly, without
    /// raising each level to at least 2. No effect without `see_through`, which already draws
    /// in `light` as given.
    pub exact_light: bool,
}
impl NameTagOptions {
    /// Describe a label's drawing; the faded text's alpha follows the text background opacity
    /// and the light of the copy in the world is raised to at least 2.
    pub fn new(see_through: bool, background: bool, light: u8) -> Self {
        Self {
            see_through,
            background,
            light,
            faded_alpha: None,
            exact_light: false,
        }
    }

    /// Draw the faded text at alpha `alpha` ([`NameTagOptions::faded_alpha`]).
    pub fn with_faded_alpha(mut self, alpha: u8) -> Self {
        self.faded_alpha = Some(alpha);
        self
    }

    /// Draw the opaque copy in `light` exactly ([`NameTagOptions::exact_light`]).
    pub fn with_exact_light(mut self) -> Self {
        self.exact_light = true;
        self
    }
}

/// Order-independent transparency (OIT) sub-passes in vanilla 26.3 (`OitStage`), in execution
/// order.
// exhaustive: vanilla 26.3's closed three-pass OitStage enum
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// no wire: local OIT sub-pass selector.
pub enum OitStage {
    /// First OIT pass: records depth bounds for translucent fragments (`OitStage.DEPTH_BOUNDS`).
    DepthBounds,
    /// Second OIT pass: computes transmittance coefficients (`OitStage.TRANSMITTANCE`).
    Transmittance,
    /// Third OIT pass: accumulates weighted translucent colour (`OitStage.ACCUMULATE`).
    Accumulate,
}

/// Where in the world's frame a stage runs: all 9 NeoForge 26.3
/// `RenderLevelStageEvent` stages, listed in engine order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local world render-stage selector.
pub enum RenderLevelStage {
    /// NeoForge's `RenderLevelStageEvent.AfterSky` in `LevelRenderer.addSkyPass`. Fires once
    /// immediately after the engine or classic pack's sky work, even if it draws nothing visible.
    /// Absent for a dimension without a skybox, lava or powder-snow camera fog, or a camera mob
    /// effect that blocks the sky. A classic pack that owns the disc or celestial programs
    /// completes the logical pass after its world import; the engine still draws unclaimed
    /// portions. Provides no active render pass.
    AfterSky,
    /// After solid and cutout chunk geometry, before opaque entities and block entities
    /// (`RenderLevelStageEvent.AfterOpaqueBlocks`). When a classic shader pack owns terrain with a
    /// float-blendable `colortex0`, the pack's opaque terrain pass supplies this boundary.
    AfterOpaqueBlocks,
    /// After opaque entities, block entities and particles
    /// (`RenderLevelStageEvent.AfterOpaqueFeatures`). With a classic pack that owns
    /// feature drawing, its opaque group owns this boundary.
    AfterOpaqueFeatures,
    /// Classic transparency only: after translucent entities and block entities
    /// (`RenderLevelStageEvent.AfterTranslucentFeatures`). On the engine route this precedes
    /// translucent chunk geometry. A classic pack supplies the boundary when its own
    /// translucent feature drawing finishes.
    AfterTranslucentFeatures,
    /// Classic transparency only: after translucent chunk geometry such as water and stained
    /// glass, before translucent particles (`RenderLevelStageEvent.AfterTranslucentBlocks`). When
    /// a classic shader pack owns terrain/water with a float-blendable `colortex0`, the pack's
    /// translucent terrain pass supplies this boundary.
    AfterTranslucentBlocks,
    /// Classic transparency only: after translucent particles, before weather
    /// (`RenderLevelStageEvent.AfterTranslucentParticles`).
    AfterTranslucentParticles,
    /// Classic transparency only: after rain and snow, before the world border
    /// (`RenderLevelStageEvent.AfterWeather`). When a classic shader pack owns weather with a
    /// float-blendable `colortex0`, the pack's weather pass supplies this boundary.
    AfterWeather,
    /// Improved transparency (OIT) only: fired once per [`OitStage`] after translucent chunk
    /// geometry, translucent features, the world border and weather
    /// (`RenderLevelStageEvent.OitTranslucent`).
    OitTranslucent(OitStage),
    /// After the level finishes rendering, before the engine's first-person hand overlay
    /// (`RenderLevelStageEvent.AfterLevel`). With a classic shader pack this follows the pack's
    /// world passes (including its own hand), before pack composition. Provides no active
    /// render pass.
    AfterLevel,
}
impl RenderLevelStage {
    /// The engine's stage order when classic transparency is active (`!view.improved_transparency`).
    /// A classic shader pack can skip engine stages or provide its own sky, block, feature, and
    /// weather boundaries on a float-blendable `colortex0`.
    pub const CLASSIC_ORDER: &[Self] = &[
        Self::AfterSky,
        Self::AfterOpaqueBlocks,
        Self::AfterOpaqueFeatures,
        Self::AfterTranslucentFeatures,
        Self::AfterTranslucentBlocks,
        Self::AfterTranslucentParticles,
        Self::AfterWeather,
        Self::AfterLevel,
    ];

    /// The stages fired in frame order when improved transparency (OIT) is active
    /// (`view.improved_transparency`).
    pub const OIT_ORDER: &[Self] = &[
        Self::AfterSky,
        Self::AfterOpaqueBlocks,
        Self::AfterOpaqueFeatures,
        Self::OitTranslucent(OitStage::DepthBounds),
        Self::OitTranslucent(OitStage::Transmittance),
        Self::OitTranslucent(OitStage::Accumulate),
        Self::AfterLevel,
    ];

    /// The engine's stage order for the active transparency mode. Classic shader packs can
    /// skip stages or complete their sky, block, feature, and weather passes after the engine
    /// world pass.
    pub const fn frame_order(improved_transparency: bool) -> &'static [Self] {
        if improved_transparency {
            Self::OIT_ORDER
        } else {
            Self::CLASSIC_ORDER
        }
    }

    /// Whether this stage has an active render pass (`AfterSky` and `AfterLevel` do not).
    pub const fn has_render_pass(self) -> bool {
        !matches!(self, Self::AfterSky | Self::AfterLevel)
    }

    /// The OIT sub-pass stage when this is [`Self::OitTranslucent`], or `None` otherwise.
    pub const fn oit_stage(self) -> Option<OitStage> {
        match self {
            Self::OitTranslucent(stage) => Some(stage),
            _ => None,
        }
    }
}

// no wire: borrows the active stage's pass surface for the duration of a render_level_stage callback.
// exhaustive: covers the three engine pass kinds for a render stage callback
/// The drawing capability active for one [`RenderLevelStage`] callback.
pub enum StagePass<'a> {
    /// No active render pass (`RenderLevelStage::AfterSky`, `RenderLevelStage::AfterLevel`).
    None,
    /// Standard world colour/depth pass (`AfterOpaqueBlocks`, `AfterOpaqueFeatures`,
    /// `AfterTranslucentFeatures`, `AfterTranslucentBlocks`, `AfterTranslucentParticles`,
    /// `AfterWeather`).
    World(&'a mut dyn crate::WorldPassDrawContext),
    /// Order-independent transparency sub-pass (`RenderLevelStage::OitTranslucent`).
    Oit(OitStage, &'a mut dyn crate::WorldOitPassDrawContext),
}

// no wire: in-process frame section view lent during world rendering.
/// One visible chunk section in the current world view (NeoForge 26.3's `IRenderableSection`).
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RenderableSection {
    /// World-space block coordinates of the section's minimum corner.
    pub origin: [i32; 3],
    /// World-space axis-aligned bounding box `[min, max]` of the section in blocks.
    pub bounds: [[f64; 3]; 2],
    /// Whether the compiled section currently holds no rendered geometry.
    pub empty: bool,
}
impl RenderableSection {
    /// Describe one visible chunk section in the current frame.
    pub const fn new(origin: [i32; 3], bounds: [[f64; 3]; 2], empty: bool) -> Self {
        Self {
            origin,
            bounds,
            empty,
        }
    }
}

// no wire: borrows frame camera state and visible sections for the duration of a render callback.
/// Read-only frame state and visible chunk sections lent for one world view.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct WorldRenderView<'a> {
    /// Frame camera state (position, eye height, rotation, projection and dimension).
    pub camera: &'a crate::Camera,
    /// The camera entity's interpolated eye position (`Entity.getEyePosition`), or `None` when
    /// no camera entity exists (screen scenes, title screen).
    pub camera_entity_eye: Option<[f64; 3]>,
    /// World partial tick (`alpha`), distinct from `camera.partial_tick` while paused or frozen.
    pub partial_tick: f32,
    /// Level game time in ticks.
    pub game_time: i64,
    /// The client's animation clock in ticks (`BlockEntityDrawContext::animation_time`).
    pub animation_time: f64,
    /// Whether vanilla 26.3 order-independent transparency (improved transparency) is active this frame.
    pub improved_transparency: bool,
    /// Visible chunk sections in the current view (NeoForge 26.3 `IRenderableSection` list).
    pub sections: &'a [RenderableSection],
}
impl<'a> WorldRenderView<'a> {
    /// Borrow the frame camera and visible sections for one world view.
    pub fn new(
        camera: &'a crate::Camera,
        camera_entity_eye: Option<[f64; 3]>,
        partial_tick: f32,
        game_time: i64,
        animation_time: f64,
        improved_transparency: bool,
        sections: &'a [RenderableSection],
    ) -> Self {
        Self {
            camera,
            camera_entity_eye,
            partial_tick,
            game_time,
            animation_time,
            improved_transparency,
            sections,
        }
    }

    /// Camera position in world-space blocks.
    pub fn camera_position(&self) -> [f64; 3] {
        self.camera.position
    }

    /// Camera yaw, pitch and roll in degrees, after camera angle hooks.
    pub fn camera_rotation(&self) -> [f32; 3] {
        [self.camera.yaw, self.camera.pitch, self.camera.roll]
    }

    /// Unit vector pointing in the camera's forward look direction (`Camera.getLookVector`),
    /// derived from `camera.yaw`, `camera.pitch` and `camera.roll` matching vanilla 26.3's
    /// `Camera.setRotation`.
    pub fn camera_forward(&self) -> [f32; 3] {
        camera_basis_transform(
            self.camera.yaw,
            self.camera.pitch,
            self.camera.roll,
            [0.0, 0.0, -1.0],
        )
    }

    /// Unit vector pointing in the camera's local up direction (`Camera.getUpVector`),
    /// derived from `camera.yaw`, `camera.pitch` and `camera.roll` matching vanilla 26.3's
    /// `Camera.setRotation`.
    pub fn camera_up(&self) -> [f32; 3] {
        camera_basis_transform(
            self.camera.yaw,
            self.camera.pitch,
            self.camera.roll,
            [0.0, 1.0, 0.0],
        )
    }

    /// Unit vector pointing in the camera's local left direction (`Camera.getLeftVector`),
    /// derived from `camera.yaw`, `camera.pitch` and `camera.roll` matching vanilla 26.3's
    /// `Camera.setRotation`.
    pub fn camera_left(&self) -> [f32; 3] {
        camera_basis_transform(
            self.camera.yaw,
            self.camera.pitch,
            self.camera.roll,
            [-1.0, 0.0, 0.0],
        )
    }

    /// Camera-relative model-view matrix in column-major order.
    pub fn model_view_matrix(&self) -> [f32; 16] {
        self.camera.view
    }

    /// Reversed-Z projection matrix in column-major order.
    pub fn projection_matrix(&self) -> [f32; 16] {
        self.camera.projection
    }
}

/// Vanilla `Camera.setRotation` (`Quaternionf.rotationYXZ(PI - yaw, -pitch, -roll)`) applied to
/// a local camera basis vector (`FORWARDS`, `UP`, `LEFT`), matching `sunrise-client-render::camera`
/// and JOML's non-FMA trig paths.
fn camera_basis_transform(yaw: f32, pitch: f32, roll: f32, v: [f32; 3]) -> [f32; 3] {
    use sunrise_core::geometry::trig::DEG_TO_RAD;
    const PI: f32 = std::f32::consts::PI;
    const TAU: f32 = std::f32::consts::TAU;
    const HALF_PI: f32 = std::f32::consts::FRAC_PI_2;

    let joml_sin = |rad: f32| sunrise_core::jmath::sin(f64::from(rad)) as f32;
    let joml_cos = |sin: f32, angle: f32| {
        let cos = (1.0 - sin * sin).sqrt();
        let a = angle + HALF_PI;
        let mut b = a - (a / TAU) as i32 as f32 * TAU;
        if f64::from(b) < 0.0 {
            b += TAU;
        }
        if b >= PI { -cos } else { cos }
    };

    let angle_y = PI - yaw * DEG_TO_RAD;
    let angle_x = -pitch * DEG_TO_RAD;
    let angle_z = if roll == 0.0 { 0.0 } else { -roll * DEG_TO_RAD };
    let sx = joml_sin(angle_x * 0.5);
    let cx = joml_cos(sx, angle_x * 0.5);
    let sy = joml_sin(angle_y * 0.5);
    let cy = joml_cos(sy, angle_y * 0.5);
    let sz = joml_sin(angle_z * 0.5);
    let cz = joml_cos(sz, angle_z * 0.5);
    let qx = cy * sx;
    let qy = sy * cx;
    let qz = sy * sx;
    let qw = cy * cx;
    let x = qx * cz + qy * sz;
    let y = qy * cz - qx * sz;
    let z = qw * sz - qz * cz;
    let w = qw * cz + qz * sz;

    let [vx, vy, vz] = v;
    let xx = x * x;
    let yy = y * y;
    let zz = z * z;
    let ww = w * w;
    let xy = x * y;
    let xz = x * z;
    let yz = y * z;
    let xw = x * w;
    let zw = z * w;
    let yw = y * w;
    let k = 1.0 / (xx + yy + zz + ww);
    [
        (xx - yy - zz + ww) * k * vx + (2.0 * (xy - zw) * k * vy + 2.0 * (xz + yw) * k * vz),
        2.0 * (xy + zw) * k * vx + ((yy - xx - zz + ww) * k * vy + 2.0 * (yz - xw) * k * vz),
        2.0 * (xz - yw) * k * vx + (2.0 * (yz + xw) * k * vy + (zz - xx - yy + ww) * k * vz),
    ]
}

#[cfg(test)]
mod tests {
    use crate::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn world_draw_values_are_not_wire_values() {
        // If a draw type implements Wire, there are two possible marker types for it
        // and the compiler rejects the ambiguous assertion.
        trait Local<Marker> {
            fn assert_local() {}
        }
        impl<T> Local<()> for T {}
        impl<T: crate::wire::Wire> Local<u8> for T {}

        let _ = <DrawError as Local<_>>::assert_local;
        let _ = <RenderLevelStage as Local<_>>::assert_local;
        let _ = <OitStage as Local<_>>::assert_local;
        let _ = <WorldDepth as Local<_>>::assert_local;
        let _ = <WorldLine as Local<_>>::assert_local;
        let _ = <WorldQuad as Local<_>>::assert_local;
        let _ = <NameTagOptions as Local<_>>::assert_local;
    }

    static CALLS: AtomicUsize = AtomicUsize::new(0);
    static PREPARE_CALLS: AtomicUsize = AtomicUsize::new(0);
    struct Listener;
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn prepare_render_buffers(
            &self,
            _: &dyn ClientHost,
            event: &mut PrepareRenderBuffersEvent<'_>,
        ) {
            assert_eq!(event.view.game_time, 42);
            assert_eq!(event.view.partial_tick, 0.5);
            assert!(event.view.improved_transparency);
            assert_eq!(event.view.camera_position(), [1.0, 2.0, 3.0]);
            assert_eq!(event.view.sections.len(), 1);
            assert_eq!(event.view.sections[0].origin, [16, 64, -32]);
            PREPARE_CALLS.fetch_add(1, Ordering::Relaxed);
        }
        fn submit_custom_geometry(
            &self,
            _: &dyn ClientHost,
            event: &mut SubmitCustomGeometryEvent<'_>,
        ) {
            assert_eq!(event.view.game_time, 42);
            assert_eq!(event.view.camera_position(), [1.0, 2.0, 3.0]);
            assert_eq!(event.view.sections.len(), 1);
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
    }
    struct Empty;
    #[mod_api::__subscriptions]
    impl Hooks for Empty {}
    impl Host for Empty {}
    mod_api::impl_host_areas!(Empty);
    impl ClientHost for Empty {}
    mod_api::impl_client_host_areas!(Empty);
    impl WorldDrawContext for Empty {}
    impl RenderLevelStageContext for Empty {}
    impl PrepareRenderBuffersContext for Empty {
        fn lines(
            &mut self,
            _: RenderLevelStage,
            _: &[WorldLine],
        ) -> Result<PreparedWorldDraw, DrawError> {
            Err(DrawError::NoRenderPass)
        }
        fn quads(
            &mut self,
            _: RenderLevelStage,
            _: &[WorldQuad],
        ) -> Result<PreparedWorldDraw, DrawError> {
            Err(DrawError::NoRenderPass)
        }
    }
    #[test]
    fn render_level_stage_orders_and_capabilities_match_neoforge() {
        assert_eq!(
            RenderLevelStage::frame_order(false),
            RenderLevelStage::CLASSIC_ORDER
        );
        assert_eq!(
            RenderLevelStage::frame_order(true),
            RenderLevelStage::OIT_ORDER
        );
        assert!(!RenderLevelStage::AfterSky.has_render_pass());
        assert!(!RenderLevelStage::AfterLevel.has_render_pass());
        for &stage in RenderLevelStage::CLASSIC_ORDER
            .iter()
            .chain(RenderLevelStage::OIT_ORDER.iter())
        {
            let expected_pass = !matches!(
                stage,
                RenderLevelStage::AfterSky | RenderLevelStage::AfterLevel
            );
            assert_eq!(stage.has_render_pass(), expected_pass);
        }
        assert_eq!(
            RenderLevelStage::OitTranslucent(OitStage::Transmittance).oit_stage(),
            Some(OitStage::Transmittance)
        );
        assert_eq!(RenderLevelStage::AfterOpaqueBlocks.oit_stage(), None);
        let mut ctx = Empty;
        assert!(!ctx.has_render_pass());
        assert!(matches!(ctx.pass(), StagePass::None));
        assert!(ctx.world_pass().is_none());
        assert!(ctx.oit_pass().is_none());
    }
    #[test]
    fn world_geometry_dispatch_skips_unsubscribed_and_disabled_mods() {
        install(
            vec![
                ModEntry::new("geometry", Box::new(Listener)),
                ModEntry::new("empty", Box::new(Empty)),
            ]
            .into(),
        );
        let camera = Camera {
            position: [1.0, 2.0, 3.0],
            eye_height: 1.62,
            partial_tick: 0.25,
            yaw: 15.0,
            pitch: -10.0,
            roll: 0.0,
            fov_degrees: 70.0,
            near: 0.05,
            far: 128.0,
            view: [0.0; 16],
            projection: [0.0; 16],
            dimension: "minecraft:overworld".into(),
        };
        let sections = [RenderableSection::new(
            [16, 64, -32],
            [[16.0, 64.0, -32.0], [32.0, 80.0, -16.0]],
            false,
        )];
        let view = WorldRenderView::new(
            &camera,
            Some([1.0, 3.62, 3.0]),
            0.5,
            42,
            100.25,
            true,
            &sections,
        );
        dispatch_prepare_render_buffers(&Empty, &view, &mut Empty);
        dispatch_submit_custom_geometry(&Empty, &view, &mut Empty);
        assert_eq!(PREPARE_CALLS.load(Ordering::Relaxed), 1);
        assert_eq!(CALLS.load(Ordering::Relaxed), 1);
        assert!(set_mod_enabled("geometry", false));
        dispatch_prepare_render_buffers(&Empty, &view, &mut Empty);
        dispatch_submit_custom_geometry(&Empty, &view, &mut Empty);
        assert_eq!(PREPARE_CALLS.load(Ordering::Relaxed), 1);
        assert_eq!(CALLS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn world_render_view_camera_basis_matches_vanilla_rotation_and_roll() {
        let make = |yaw: f32, pitch: f32, roll: f32| Camera {
            position: [1.0, 2.0, 3.0],
            eye_height: 1.62,
            partial_tick: 0.25,
            yaw,
            pitch,
            roll,
            fov_degrees: 70.0,
            near: 0.05,
            far: 128.0,
            view: [0.0; 16],
            projection: [0.0; 16],
            dimension: "minecraft:overworld".into(),
        };
        let assert_vec = |got: [f32; 3], expected: [f32; 3]| {
            for i in 0..3 {
                assert!(
                    (got[i] - expected[i]).abs() < 1e-5,
                    "got {got:?}, expected {expected:?}"
                );
            }
        };
        // Yaw 0 faces south (+Z), up is +Y, left is east (+X).
        let south = make(0.0, 0.0, 0.0);
        let view = WorldRenderView::new(&south, Some([1.0, 3.62, 3.0]), 0.5, 42, 10.5, false, &[]);
        let copied = view;
        assert_eq!(copied.camera_rotation(), [0.0, 0.0, 0.0]);
        assert_eq!(copied.camera_entity_eye, Some([1.0, 3.62, 3.0]));
        assert_eq!(copied.animation_time, 10.5);
        assert_vec(view.camera_forward(), [0.0, 0.0, 1.0]);
        assert_vec(view.camera_up(), [0.0, 1.0, 0.0]);
        assert_vec(view.camera_left(), [1.0, 0.0, 0.0]);

        // Yaw 90 faces west (-X), up is +Y, left is south (+Z).
        let west = make(90.0, 0.0, 0.0);
        let view = WorldRenderView::new(&west, None, 0.0, 0, 0.0, false, &[]);
        assert_vec(view.camera_forward(), [-1.0, 0.0, 0.0]);
        assert_vec(view.camera_up(), [0.0, 1.0, 0.0]);
        assert_vec(view.camera_left(), [0.0, 0.0, 1.0]);

        // Pitch 90 looks straight down (-Y); up points south (+Z), left stays east (+X).
        let down = make(0.0, 90.0, 0.0);
        let view = WorldRenderView::new(&down, None, 0.0, 0, 0.0, false, &[]);
        assert_vec(view.camera_forward(), [0.0, -1.0, 0.0]);
        assert_vec(view.camera_up(), [0.0, 0.0, 1.0]);
        assert_vec(view.camera_left(), [1.0, 0.0, 0.0]);

        // Roll 90 clockwise about view axis: facing south (+Z), up tilts west (-X), left tilts up (+Y).
        let rolled = make(0.0, 0.0, 90.0);
        let view = WorldRenderView::new(&rolled, None, 0.0, 0, 0.0, false, &[]);
        assert_eq!(view.camera_rotation(), [0.0, 0.0, 90.0]);
        assert_vec(view.camera_forward(), [0.0, 0.0, 1.0]);
        assert_vec(view.camera_up(), [-1.0, 0.0, 0.0]);
        assert_vec(view.camera_left(), [0.0, 1.0, 0.0]);
    }
}
