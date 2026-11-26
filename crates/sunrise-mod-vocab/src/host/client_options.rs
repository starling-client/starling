//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The client host's methods of the `client_options` area, implemented by the game and reached through [`crate::ClientHost`].
#[sunrise_mod_api_macros::host_area]
pub trait ClientOptionsClientHost {
    /// Side: client.
    /// A snapshot of the current input preferences.
    fn input_options(&self) -> ClientInputOptions;
    /// Side: client.
    /// A snapshot of the current graphics preferences.
    fn graphics_options(&self) -> ClientGraphicsOptions;
    /// Side: client.
    /// Turns VSync on or off for this session only, without writing `options.txt`, and
    /// reconfigures the window's surface at once, as changing the option does
    /// (`Options.enableVsync`, `Minecraft.invalidateSurfaceConfiguration`). `None` goes back to
    /// the player's saved option. The options screen keeps showing and saving the player's own
    /// value; `graphics_options().vsync` reads the one in effect.
    fn set_session_vsync(&mut self, vsync: Option<bool>);
    /// Side: client.
    /// Sets how clouds are drawn for this session only, without writing `options.txt`
    /// (`Options.cloudStatus`); `None` goes back to the player's saved option. The options screen
    /// keeps showing and saving the player's own value. The next frame draws with it.
    fn set_session_clouds(&mut self, clouds: Option<CloudStatus>);
    /// Side: client.
    /// Turns view bobbing on or off for this session only, without writing `options.txt`
    /// (`Options.bobView`); `None` goes back to the player's saved option. The options screen
    /// keeps showing and saving the player's own value; `graphics_options().view_bobbing` reads
    /// the one in effect. The next frame's camera bobs, or not, with it.
    fn set_session_view_bobbing(&mut self, view_bobbing: Option<bool>);
    /// Side: client.
    /// How clouds are drawn now: the session's choice (`set_session_clouds`), else the option.
    fn cloud_status(&self) -> CloudStatus;
    /// Side: client.
    /// A snapshot of the current GUI and accessibility preferences. Its
    /// `gui_scale` is only the option, where 0 means automatic; read
    /// [`Self::gui_scale`] for the scale the window applies.
    fn gui_options(&self) -> ClientGuiOptions;
    /// Side: client.
    /// The GUI scale in effect (`Window.getGuiScale`), at least 1; none on a
    /// server or when no window exists. Vanilla's `Window.calculateScale`
    /// resolves it from the GUI Scale option and the window size: the option is
    /// a maximum (0 = automatic, not a guarantee), and with the Unicode font
    /// forced an odd result is raised by one. This read returns the scale the
    /// window applies, never a separate recomputation. Divide
    /// [`Self::window_size`] by this scale, rounding up, for the GUI-scaled
    /// width and height (`Window.getGuiScaledWidth/Height`); draw callbacks
    /// read their GUI dimensions from [`DrawContext::size`].
    fn gui_scale(&self) -> Option<u32>;
    /// Side: client.
    /// The window's width and height in framebuffer pixels (`Window.getWidth`
    /// and `Window.getHeight`), whatever the GUI scale; none on a server or
    /// when no window exists. Readable at any time: `window_resize` reports
    /// this same size, so the two always agree.
    fn window_size(&self) -> Option<[u32; 2]>;
    /// Side: client.
    /// A snapshot of the current chat preferences.
    fn chat_options(&self) -> ClientChatOptions;
    /// Side: client.
    /// A snapshot of the current audio preferences.
    fn audio_options(&self) -> ClientAudioOptions;
    /// Side: client.
    /// Whether a shader pack is currently loaded and active (Iris's
    /// `IrisApi.isShaderPackInUse()`). `false` on a server, when no renderer
    /// exists, or while no shader pack is running.
    fn shader_pack_active(&self) -> bool;
    /// Side: client.
    /// The active shader pack's cloud and shadow-pass state for the current frame, or `None`
    /// when no pack is active (the same rule as [`Self::shader_pack_active`]: `None` on the
    /// title screen and while unloading). Taken at the start of each frame's render; read it in
    /// the render callbacks to cull shadow-casting geometry against
    /// [`ShaderPackState::shadow`] and to skip clouds when [`ShaderPackState::hides_clouds`].
    /// Compare [`ShaderPackState::generation`] each frame to notice a pack loaded, switched or
    /// turned off.
    fn shader_pack_state(&self) -> Option<ShaderPackState>;
}
