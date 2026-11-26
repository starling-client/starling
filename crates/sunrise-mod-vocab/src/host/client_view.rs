//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `client_view` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ClientViewClientHost {
    /// Side: client.
    /// The HUD tick counter (`Hud.getGuiTicks`): advances once per unpaused client tick,
    /// including on the title screen, independently of server tick freeze. Wraps as a Java int.
    fn gui_ticks(&self) -> i32;
    /// Side: client.
    /// How far above the bottom edge the HUD's status rows reach, left and right in GUI pixels
    /// (NeoForge's `Hud.leftHeight` and `rightHeight`), as they stand when the current HUD layer
    /// starts: both are 39 (the hotbar and experience bar) before `minecraft:player_health`,
    /// and each vanilla row adds its height as it draws (health and armour on the left, food,
    /// a vehicle's hearts and air on the right). A layer placed `Below` a vanilla row sees the
    /// height that row draws at, and one placed `Above` it the height after it. A vanilla row
    /// that is hidden, canceled or replaced adds nothing. None outside the HUD layer pass.
    fn gui_status_heights(&self) -> Option<[i32; 2]>;
    /// Side: client.
    /// Sets the HUD status columns' heights, left and right in GUI pixels above the bottom
    /// edge, as assigning NeoForge's `Hud.leftHeight` and `rightHeight` does: the vanilla rows
    /// that draw after the current layer start from them, and add their own height on top. A
    /// layer placed `Below` `minecraft:vehicle_health` that raises the right column moves the
    /// mount's hearts and the air row up; a wrapper that sets them before `draw_previous` moves
    /// the row it wraps. Rows already drawn stay where they are, and both columns go back to 39
    /// at the start of each HUD pass. `gui_status_heights` reads the new heights at once. False,
    /// setting nothing, outside the HUD layer pass.
    fn set_gui_status_heights(&self, heights: [i32; 2]) -> bool;
    /// Side: client.
    /// Read effective HUD visibility independently of first-person hands.
    fn hud_mode(&self) -> HudMode;
    /// Side: client.
    /// Select HUD and hand visibility, including after vanilla F1 input handling.
    /// Other first-person hand restrictions and render cancellation still apply.
    /// This does not change the player's F1 preference for mandatory mod-error diagnostics.
    fn set_hud_mode(&mut self, mode: HudMode);
    /// Side: client.
    /// Shows `line` over this window's hotbar (the action bar), as `Hud.setOverlayMessage`
    /// does: it replaces the current line, empty text included, and restarts the stock
    /// overlay timer and fade (`Hud.tick`, `Hud.extractOverlayMessage`). `animate_color`
    /// cycles the colour as a jukebox's "Now Playing" line does. One slot: a server's
    /// action-bar packet or a jukebox message replaces a mod's line, and a mod's line
    /// replaces theirs. Nothing joins chat or its history, no chat event fires, nothing is
    /// narrated and nothing is sent. Text the boundary refuses is an error and the previous
    /// line stays.
    fn set_overlay_message(&mut self, line: Text, animate_color: bool)
    -> Result<(), TextFormError>;
    /// Side: client.
    /// The window's camera type (`Options.getCameraType`), including its latest pending request.
    fn camera_type(&self) -> CameraType;
    /// Side: client.
    /// Request the window's camera type without cycling, also from shared-host callbacks.
    /// The last request wins and [`Self::camera_type`] reports it immediately. Requests before
    /// or during `compute_camera_angles` apply before that frame's detached camera placement,
    /// FOV and spectator post effects. Requests during `camera_position` re-align an entity-bound
    /// camera with the already-computed angles, keeping only coordinates explicitly pinned by listeners.
    /// The selected custom camera is not re-aligned. Later requests apply at the next frame's
    /// camera extraction. No cycle hook is emitted.
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
    /// The tick rate state of the world this window shows (`ClientLevel.tickRateManager()`):
    /// its rate, whether it is frozen and the frozen steps left to run, as the server last
    /// sent them (`ClientPacketListener.handleTickingState` / `handleTickingStep`). It is there
    /// from `Login`, when the client level is made, before the player is placed; none outside
    /// a client world (the title screen, before login, or while reconfiguring).
    fn tick_rate(&self) -> Option<ClientTickRate>;
    /// Side: client.
    /// The first block along the view within `distance`, in loaded chunks only;
    /// `None` on a miss or a nonpositive or non-finite `distance`.
    fn raycast(&self, distance: f32) -> Option<Hit>;
    /// Side: client.
    /// The first block along the view across loaded chunks only, and its distance; `None` on a miss.
    /// The shape clip does not depend on how far away other chunks are loaded.
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
    /// Side: client.
    /// The vehicle the local player controls that can jump now (`LocalPlayer.jumpableVehicle`):
    /// the player is its controlling passenger (`getControlledVehicle`: the first one, on its
    /// saddle) and it can jump (`PlayerRideableJumping.canJump`): a saddled horse, donkey,
    /// mule, nautilus or camel, the camel neither sitting nor getting up or down; a pig or a
    /// strider never jumps. The HUD's jump bar makes the same decision. None when the player
    /// controls no such vehicle (a second passenger included), or outside a client world.
    fn jumpable_vehicle(&self) -> Option<EntityRef>;
    /// Side: client.
    /// How far the local player has charged its mount's jump (`LocalPlayer.getJumpRidingScale`),
    /// 0.0 to 1.0: the value the HUD's jump bar draws and `Hud.willPrioritizeJumpInfo` tests
    /// (`> 0`). It rises while the jump key is held on a [`Self::jumpable_vehicle`] and drops
    /// back to 0.0 when the jump is released and its cooldown runs out. 0.0 once a client tick
    /// has run with the player controlling no mount that can jump (that tick resets it), and
    /// outside a client world. Read-only: the jump key
    /// drives it, and the server checks the jump it is sent.
    fn jump_riding_scale(&self) -> f32;
    /// Side: client.
    /// Whether the server tracks any waypoint for this client
    /// (`ClientWaypointManager.hasWaypoints`), the waypoints the locator bar draws. The HUD's
    /// contextual bar reads the same fact. Which bar the HUD shows is not exposed: vanilla's
    /// `Hud.nextContextualInfoState` is private.
    fn has_waypoints(&self) -> bool;
    /// Side: client.
    /// The ticks left before the local player's next block starts breaking
    /// (`MultiPlayerGameMode.destroyDelay`): 5 after a block breaks or creative's instant break
    /// starts, then one less for each tick the attack key keeps continuing the destroy
    /// (`continueDestroyBlock`), until 0 lets the next block start. While it is above 0 a
    /// held attack starts no break. Read-only: vanilla has no public setter. 0 outside a
    /// client world.
    fn mining_destroy_delay(&self) -> u32;
}
