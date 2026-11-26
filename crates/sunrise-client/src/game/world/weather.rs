//! Weather and sky light for the player's level: whether it can have weather, the rain and thunder
//! levels, where it rains or snows (`Level.isRainingAt`, the motion-blocking height), the weather's
//! effects each tick and `Level.getSkyDarken` with `isDarkOutside`.
use super::*;

/// `ClientLevel.getPrecipitationAt`: a loaded column's biome and the level's sea level
/// decide whether it snows, rains, or has no precipitation.
pub(super) fn precipitation_at(
    world: &World,
    biomes: &Biomes,
    zoom: BiomeZoom,
    pos: BlockPos,
    sea_level: i32,
) -> Option<bool> {
    if !world.chunks.contains_key(&(pos.x >> 4, pos.z >> 4)) {
        return None;
    }
    // `LevelReader.getBiome` zooms first. If that quart's chunk is missing,
    // `ClientLevel.getUncachedNoiseBiome` supplies plains.
    let [x, y, z] = zoom.quart_at_block(pos).map(|q| q << 2);
    let id = world
        .noise_biome(x, y, z)
        .unwrap_or_else(|| biomes.id("minecraft:plains"));
    biomes.get(id).snows_at(pos, sea_level)
}

/// What `Level.precipitationAt` reads of the level in place: it rains (the rain level above 0.2),
/// the sky light is 15, the motion-blocking height, and the biome's precipitation there
/// (`ClientLevel.getPrecipitationAt`'s zoomed biome; snow is `Some(true)` there).
struct GamePrecipitation<'a, 'g>(&'a Game<'g>);

impl sunrise_world::level::weather::PrecipitationReads for GamePrecipitation<'_, '_> {
    fn is_raining(&self) -> bool {
        self.0.weather_levels(1.0).0 > 0.2
    }
    fn can_see_sky(&self, p: BlockPos) -> bool {
        self.0.world.light(p.x, p.y, p.z) >> 4 == 15
    }
    fn motion_blocking_y(&self, p: BlockPos) -> i32 {
        self.0.rain_height(p.x, p.z)
    }
    fn biome_precipitation(&self, p: BlockPos) -> sunrise_world::level::PrecipitationKind {
        use sunrise_world::level::PrecipitationKind;
        let game = self.0;
        match precipitation_at(
            &game.world,
            world_biomes!(game),
            game.streaming.biome_zoom,
            p,
            game.weather_sea_level(),
        ) {
            Some(true) => PrecipitationKind::Snow,
            Some(false) => PrecipitationKind::Rain,
            None => PrecipitationKind::None,
        }
    }
}

impl Game<'_> {
    /// `MinecraftServer.setWeatherParameters`: the dimension names the caller's origin,
    /// but the weather timers and flags belong to the server. A veto skips only this write.
    pub(in crate::game) fn set_weather_parameters(
        &mut self,
        origin_dimension: &str,
        clear_ticks: i32,
        weather_ticks: i32,
        raining: bool,
        thundering: bool,
    ) -> bool {
        let mut cancel = false;
        self.native_weather_change(
            origin_dimension,
            clear_ticks,
            weather_ticks,
            raining,
            thundering,
            &mut cancel,
        );
        if cancel {
            return false;
        }

        let weather = &mut self.world.weather;
        weather.clear_time = clear_ticks;
        weather.rain_time = weather_ticks;
        weather.thunder_time = weather_ticks;
        weather.raining = raining;
        weather.thundering = thundering;
        true
    }

    /// `ServerLevel.advanceWeatherCycle`: tick this level's interpolants against the
    /// shared server timers and flags, even when its chunks and entities are idle.
    pub(super) fn tick_level_weather(&mut self, dim: Dim) {
        let parameters = self.world.weather;
        let world = if dim == self.dim {
            &mut self.world
        } else if let Some(parked) = self.levels.parked_mut(dim) {
            &mut parked.world
        } else {
            return;
        };
        if !world.shape.can_have_weather(dim == Dim::End) {
            return;
        }
        if dim != self.dim {
            world.weather.copy_parameters_from(parameters);
        }
        if self
            .ui
            .rules
            .bool_at(sunrise_client::gamerules::vanilla::ADVANCE_WEATHER)
        {
            world.weather.tick_timers(&mut world.random);
        }
        world.weather.tick_levels();
        if dim != self.dim {
            let parameters = world.weather;
            self.world.weather.copy_parameters_from(parameters);
        }
    }

    /// The joined `ClientLevel`'s sea level, or the server level's generator setting.
    pub(super) fn weather_sea_level(&self) -> i32 {
        self.remote
            .as_ref()
            .map_or_else(|| self.dim.sea_level(), |remote| remote.sea_level)
    }

    /// `Level.isDarkOutside`.
    pub(super) fn dark_outside(&self) -> bool {
        !self.dimension_type().has_fixed_time && self.sky_darken() >= 4
    }

    pub(super) fn can_have_weather(&self) -> bool {
        self.world.shape.can_have_weather(self.dim == Dim::End)
    }

    /// The rain and thunder levels (`Level.getRainLevel`, `getThunderLevel`):
    /// the world's, where the level can have weather, else none.
    pub(super) fn weather_levels(&self, partial: f32) -> (f32, f32) {
        if self.can_have_weather() {
            self.world.weather.levels(partial)
        } else {
            (0.0, 0.0)
        }
    }

    pub(super) fn sky_darken(&self) -> i32 {
        if self.dim == Dim::Nether {
            return 11;
        }
        if self.dim == Dim::End {
            let clock = |name: &str| {
                if let Some(remote) = self.remote.as_ref() {
                    remote.named_clock_time(name)
                } else {
                    self.command_clock_time(name)
                }
            };
            return sunrise_client::environment::sky_darken_with_clock(
                self.dim_environment(),
                0.0,
                0.0,
                &clock,
            );
        }
        let (rain, thunder) = self.weather_levels(1.0);
        // Sampled once per (dimension, time, weather): every mob asks each tick.
        let key = (
            self.dim,
            self.world.day_time,
            rain.to_bits(),
            thunder.to_bits(),
        );
        if let Some((k, darken)) = self.sky_darken_memo.get()
            && k == key
        {
            return darken;
        }
        let darken = self.sky_darken_in(&self.world, self.dim);
        self.sky_darken_memo.set(Some((key, darken)));
        darken
    }

    /// `Level.isRainingAt`: `Level.precipitationAt`
    /// (`sunrise_world::level::weather::precipitation_at`) is rain.
    pub(super) fn raining_at(&self, p: BlockPos) -> bool {
        sunrise_world::level::weather::precipitation_at(&GamePrecipitation(self), p)
            == sunrise_world::level::PrecipitationKind::Rain
    }

    /// `getHeight(Heightmap.Types.MOTION_BLOCKING, x, z)`: one above the
    /// highest block in `blocks_motion_in_heightmap` or holding a fluid.
    pub(super) fn rain_height(&self, x: i32, z: i32) -> i32 {
        render_tick::motion_blocking_height(&self.world, &self.pack.assets.properties, x, z)
    }

    pub(super) fn tick_weather_effects(&mut self) {
        if self.flash > 0 {
            self.flash -= 1;
        }
        // The rain sounds come with the rain splashes (`tick_particles`).
    }
}
