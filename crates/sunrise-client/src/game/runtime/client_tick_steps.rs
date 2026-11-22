//! The client ticks of one frame (`Minecraft.runTick`'s tick loop), after the frame has handled
//! what the server sent (`update_world`).
use super::*;

impl Game<'_> {
    /// Runs `steps` client ticks: each tells `client_tick_pre`, ticks the game unless it is
    /// paused or on the title screen (the level, its block entities' client tickers included),
    /// does the client's own per-tick work and tells `client_tick_post`. With `event_loop`, an
    /// armed tick script plays its step before the game ticks.
    pub(in crate::game) fn client_tick_steps(
        &mut self,
        steps: u32,
        event_loop: Option<&dyn EventLoopCtl>,
    ) -> Result<()> {
        if steps != 0 {
            // Client hooks run in menus too. Their Host can read the world-rest tables even
            // before a level is open, so every entry path (not only the desktop loading
            // window) must publish those tables before the first tick is dispatched.
            self.join_world_rest()?;
        }
        for _ in 0..steps {
            let started = Instant::now();
            sunrise_client::social::set_presence_status(self.presence_status());
            sunrise_client::social::tick();
            // Minecraft.tick: every client tick, paused or not, in menus too.
            self.native_client_tick_pre();
            if self.title.is_none() && !self.pause {
                if let Some(event_loop) = event_loop
                    && self.tick_script.armed()
                {
                    self.tick_script_step(event_loop);
                }
                sunrise_client::spans::time("tick", || self.tick())?;
                // Cave mood (a shader pack's playerMood) samples on a random
                // seeded from the clock: under `SUNRISE_STILL` it waits for the
                // world and draws from its tick.
                if !self.clock.still() {
                    self.tick_ambience();
                } else if self.clock.runs_normally() {
                    self.ambience.reseed(self.visual_ticks() as i64);
                    self.tick_ambience();
                }
                self.tick_fov();
            } else if self.pause {
                // Minecraft.tick still picks while the game mode and level are paused.
                self.refresh_client_pick(1.0);
            }
            self.tick_win_screen()?;
            self.native_configs_tick_client();
            // Gui.tick runs ChatListener.tick paused or not; Hud.tickCount advances
            // only when unpaused, including when there is no level on the title screen.
            if !self.pause {
                self.gui_ticks = self.gui_ticks.wrapping_add(1);
            }
            self.tick_chat_queue();
            self.tick_music();
            self.tick_sound_engine();
            // ClientTickEvent.Post, the tick's work done.
            self.native_client_tick_post();
            self.clock.record(started.elapsed());
            if let Some(report) = self.clock.take_sprint_report() {
                self.sprint_report(&report);
                self.on_tick_rate_changed();
            }
        }
        self.clock.update_animation_phase(self.texture_ticks);
        Ok(())
    }
    /// The window's own lead of a tick: debug keys and charts, the tutorial and the creative screen.
    pub(in crate::game) fn tick_client_lead(&mut self) -> Result<()> {
        self.tick_debug_keys()?;
        self.tick_debug_charts();
        self.tick_tutorial();
        if self.screen == Screen::Creative {
            self.refresh_creative_tabs();
            // CreativeModeInventoryScreen.containerTick: a player without
            // infinite materials gets the survival inventory instead.
            if !self.creative {
                if let Some(c) = self.carried.take() {
                    let _ = self.add_stack_to_inventory(c);
                }
                self.set_screen(Screen::Inventory);
            }
        }
        Ok(())
    }
}
