//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `client_view` area, implemented by the game and reached through [`Host`].
#[sunrise_mod_api_macros::host_area]
pub trait ClientViewHost {
    /// Side: both.
    /// Collision boxes in block-local coordinates with an empty collision context.
    fn block_collision(&self, pos: [i32; 3]) -> Vec<[f64; 6]>;
}
/// The client host's methods of the `client_view` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ClientViewClientHost {
    /// Side: client.
    /// The HUD tick counter (`Hud.getGuiTicks`): advances once per unpaused client tick,
    /// including on the title screen, independently of server tick freeze. Wraps as a Java int.
    fn gui_ticks(&self) -> i32;
    /// Side: client.
    /// Read effective HUD visibility independently of first-person hands.
    fn hud_mode(&self) -> HudMode;
    /// Side: client.
    /// Select HUD and hand visibility, including after vanilla F1 input handling.
    /// Other first-person hand restrictions and render cancellation still apply.
    /// This does not change the player's F1 preference for mandatory mod-error diagnostics.
    fn set_hud_mode(&mut self, mode: HudMode);
    /// Side: client.
    /// The window's camera type (`Options.getCameraType`), including its latest pending request.
    fn camera_type(&self) -> CameraType;
    /// Side: client.
    /// Request the window's camera type without cycling, also from shared-host callbacks.
    /// The last request wins and [`Self::camera_type`] reports it immediately. Requests before
    /// or during `camera_pose` apply to that frame's camera, FOV and spectator post effects;
    /// later requests apply at the next frame's camera extraction. Explicit pose overrides
    /// remain unchanged when vanilla placement is refreshed. No cycle hook is emitted.
    fn set_camera_type(&self, camera_type: CameraType);
    /// Side: client.
    /// The applied client pause state, not whether a pause screen is open.
    fn is_paused(&self) -> bool;
    /// Side: client.
    /// The clock of the world this window shows, for client mods: its game
    /// time and time of day as the server last sent them
    /// (`ClientboundSetTimePacket`), advanced each tick between packets; none
    /// on the title screen.
    fn clock(&self) -> Option<Clock>;
    /// Side: client.
    /// The first block along the view within `distance` (at most 1024), in loaded chunks only; `None` on a miss.
    fn raycast(&self, distance: f32) -> Option<Hit>;
    /// Side: client.
    /// The first block along the view within 1024 blocks, in loaded chunks only, and its distance; `None` on a miss.
    fn view_hit(&self) -> Option<(f32, Hit)>;
    /// Side: client.
    /// The most recently extracted world frame's camera, if one exists.
    /// During GUI rendering this is the current frame, including bob/spin and FOV.
    fn camera(&self) -> Option<Camera>;
    /// Side: client.
    /// World point `at` projected on this frame's screen (`GameRenderer.projectPointToScreen`):
    /// normalised device x and y, -1 to 1 across the screen (x to the right, y up), and depth z
    /// (reversed: nearer is larger). The view is the camera's rotation without bob or spin, as
    /// vanilla's. Unlike vanilla, None for a point behind the camera or a non-finite `at`; also None
    /// outside a world, before extraction, and between render frames. Supported during GUI,
    /// screen and world drawing after camera extraction. In GUI pixels of a [`DrawContext`] of size
    /// `[w, h]`: `((x + 1) / 2 * w, (1 - y) / 2 * h)`.
    fn project(&self, at: [f64; 3]) -> Option<[f32; 3]>;
    /// Side: client.
    /// The local player's ordinary item use as the client sees it: the hand, ticks used and
    /// ticks left (`isUsingItem`, `getUsedItemHand`, `getTicksUsingItem`,
    /// `getUseItemRemainingTicks`). None when the player uses no item or no client world is open.
    /// Does not report input intercepted by [`crate::ClientItemUseBehavior`]. Sends nothing.
    fn local_item_use(&self) -> Option<ItemUse>;
    /// Side: client.
    /// Turns the local player's view to `yaw` and `pitch` (degrees), as `setYRot` and `setXRot`
    /// store them: yaw as given (not wrapped; the server wraps it), pitch `% 360` then clamped to
    /// -90..90. The previous-frame angles move by the same amount, as `Entity.turn` does for the
    /// mouse, so the view doesn't jump for a frame. Sends no packet: the game's own player sync of
    /// this client tick sends the look in its one move packet, only while the camera is the player
    /// (the rotation-only packet when riding); a call after that step goes out the next tick.
    /// `NotFinite`, changing nothing, for a NaN or infinite value; `NoWorld` outside a client world.
    fn set_local_rotation(&mut self, yaw: f32, pitch: f32) -> Result<(), LocalRotationError>;
    /// Side: client.
    /// The entity the view follows (`Minecraft.getCameraEntity`): the entity a server attached
    /// the camera to, else the local player. None outside a client world.
    fn camera_entity(&self) -> Option<EntityRef>;
    /// Side: client.
    /// Each column's height by `heightmap` in the world this window shows
    /// (`LevelReader.getHeight(type, x, z)` on a client level): one above the highest block that
    /// counts on that map, from the blocks the server has sent, none for a column whose chunk this
    /// client does not hold. `Err`, reading nothing, for no client world (the title screen) or more
    /// than 65,536 columns (`MAX_COLUMNS`).
    fn client_surface_heights(
        &self,
        columns: &[(i32, i32)],
        heightmap: Heightmap,
    ) -> Result<Vec<Option<i32>>, String>;
    /// Side: client.
    /// Points the view at `entity` (`Minecraft.setCameraEntity`), as a server's camera packet
    /// does; the local player's own ref puts it back. True when the view now follows it; false,
    /// with nothing changed, for a stale ref, an entity this window is not shown, or one the
    /// camera cannot follow (only the local player and mobs). While the view is off the player,
    /// the player sends no moves (`LocalPlayer.isControlledCamera`). When the camera entity is
    /// removed, the view returns to the local player. A mod can also release the view by calling
    /// this with the player's ref; this client-only selection does not notify the server.
    fn set_camera_entity(&mut self, entity: EntityRef) -> bool;
    /// Side: client.
    /// The committed `client_pick` result (Minecraft.hitResult), computed
    /// at partial tick 1.0 before each tick's inputs, including while paused,
    /// and at the frame's partial tick before extraction. Each input reads its
    /// tick's result; the highlight reads the frame's result. Reads during
    /// dispatch see original. None before the first pick, without a client world
    /// or on a miss. Consumption never recasts or rechecks target liveness;
    /// a block broken earlier in a tick remains that tick's use target.
    fn crosshair_target(&self) -> Option<CrosshairTarget>;
}
