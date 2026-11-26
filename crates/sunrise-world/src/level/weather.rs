//! Biome weather queries for a live level: `ServerLevel.tickPrecipitation`'s
//! inputs (`Biome.shouldFreeze`, `shouldSnow`, `getPrecipitationAt`), and
//! `Level.precipitationAt` with `isRainingAt`, from the biome temperatures of the worldgen bundle.
use super::{Behaviours, LevelAccess, Precipitation, PrecipitationKind, Ticks};
use crate::{
    fluid::FluidKind,
    worldgen::biome_temperature::{BiomeTemperatureSettings, Precipitation as BiomePrecipitation},
};
use sunrise_core::{geometry::Direction, position::BlockPos};

fn sea_level<L: LevelAccess + ?Sized>(level: &L) -> i32 {
    level.sea_level()
}

impl Behaviours {
    fn biome_settings<L: LevelAccess + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
    ) -> Option<BiomeTemperatureSettings> {
        let features = self.live_features()?;
        let biome = features.biome_id(level.biome_name(pos)?)?;
        features.biome_temperature(biome)
    }
    /// `Biome.getPrecipitationAt(pos, seaLevel)`.
    fn precipitation_kind(
        settings: BiomeTemperatureSettings,
        pos: BlockPos,
        sea: i32,
    ) -> BiomePrecipitation {
        if !settings.has_precipitation {
            BiomePrecipitation::None
        } else if settings.height_adjusted(pos, sea) < 0.15 {
            BiomePrecipitation::Snow
        } else {
            BiomePrecipitation::Rain
        }
    }
    /// `getHeightmapPos(MOTION_BLOCKING, pos)`, scanning down from the
    /// world surface (air neither blocks motion nor holds a fluid).
    pub fn motion_blocking_top<L: LevelAccess + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
    ) -> BlockPos {
        let blocks = self.tag("minecraft:blocks_motion_in_heightmap");
        let top = level.world_surface(pos.x, pos.z).min(level.max_y());
        for y in (level.min_y()..top).rev() {
            let state = level.block_state(BlockPos::new(pos.x, y, pos.z));
            let fluid = self.fluid(state).kind() != FluidKind::Empty;
            if fluid || blocks.is_some_and(|t| t.contains(self.block(state))) {
                return BlockPos::new(pos.x, y + 1, pos.z);
            }
        }
        BlockPos::new(pos.x, level.min_y(), pos.z)
    }
    /// Answers for one `ServerLevel.tickPrecipitation` at the column of `pos`;
    /// None without live features or a biome.
    pub fn precipitation_info<L: LevelAccess + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
    ) -> Option<Precipitation> {
        let top = self.motion_blocking_top(level, pos);
        let below = BlockPos::new(top.x, top.y - 1, top.z);
        let settings = self.biome_settings(level, top)?;
        let sea = sea_level(level);
        let inside = |p: BlockPos| p.y >= level.min_y() && p.y < level.max_y();
        // Biome.shouldFreeze(level, below, true)
        let freezes_below = settings.height_adjusted(below, sea) < 0.15
            && inside(below)
            && level.block_light(below) < 10
            && {
                let state = level.block_state(below);
                self.block(state) == self.named.water && self.fluid(state).is_source()
            }
            && [
                Direction::West,
                Direction::East,
                Direction::North,
                Direction::South,
            ]
            .into_iter()
            .any(|d| {
                let n = super::update::rel(below, d);
                !self.is_water_fluid(self.fluid(level.block_state(n)).kind())
            });
        // Biome.shouldSnow(level, top)
        let snows_at_top = Self::precipitation_kind(settings, top, sea) == BiomePrecipitation::Snow
            && inside(top)
            && level.block_light(top) < 10
            && {
                let state = level.block_state(top);
                (self.is_air(state) || self.block(state) == self.named.snow)
                    && self.snow_survives(level, top)
            };
        let kind = match Self::precipitation_kind(settings, below, sea) {
            BiomePrecipitation::None => PrecipitationKind::None,
            BiomePrecipitation::Rain => PrecipitationKind::Rain,
            BiomePrecipitation::Snow => PrecipitationKind::Snow,
        };
        Some(Precipitation {
            top,
            freezes_below,
            snows_at_top,
            kind,
        })
    }
    /// `SnowLayerBlock.canSurvive` for a default snow layer at `pos`.
    fn snow_survives<L: LevelAccess + ?Sized>(&self, level: &L, pos: BlockPos) -> bool {
        let below_pos = BlockPos::new(pos.x, pos.y - 1, pos.z);
        let below = level.block_state(below_pos);
        let block = self.block(below);
        if self.tags.cannot_support_snow_layer.contains(block) {
            return false;
        }
        if self.tags.support_override_snow_layer.contains(block) {
            return true;
        }
        let full = match level.published_collision_shape(below_pos, below) {
            Some(shape) => super::data::analyse_shape(&shape)
                .is_ok_and(|info| info.full_faces & (1 << Direction::Up as u8) != 0),
            None => self.collision_face_full(below, Direction::Up),
        };
        full || (block == self.named.snow && self.table.int(below, self.props.layers) == 8)
    }
    /// `ServerLevel.tickPrecipitation(pos)`, one of a ticking chunk's draws: at the column's top
    /// (`getHeightmapPos(MOTION_BLOCKING, pos)`), water freezes where the biome freezes
    /// (`Biome.shouldFreeze`, rain or not), and while it rains snow piles up where the biome snows
    /// (`shouldSnow`) and the block under the top takes the precipitation
    /// (`Block.handlePrecipitation`: cauldrons fill). A level with nothing there (`precipitation`
    /// answers None) does nothing.
    ///
    /// A native mod's `[[native.mixin]]` row can replace it, as a Java mod's mixin on
    /// `tickPrecipitation` does: a cancelling `HEAD` keeps a level from freezing, snowing and
    /// filling cauldrons.
    #[sunrise_mod_api_macros::replaceable]
    pub fn tick_precipitation(
        &self,
        level: &mut dyn LevelAccess,
        ticks: &mut Ticks,
        pos: BlockPos,
    ) {
        let Some(info) = level.precipitation(pos) else {
            return;
        };
        self.precipitation_effects(level, ticks, info);
    }
    /// The effects of [`Self::tick_precipitation`] once its biome answers are known.
    pub fn precipitation_effects(
        &self,
        level: &mut dyn LevelAccess,
        ticks: &mut Ticks,
        info: Precipitation,
    ) {
        self.sim(level, ticks).precipitation_tick(info);
    }
    /// `Level.isRainingAt(pos)`: [`precipitation_at`] is rain.
    pub fn is_raining_at<L: LevelAccess + ?Sized>(&self, level: &L, pos: BlockPos) -> bool {
        precipitation_at(&LevelPrecipitation { b: self, level }, pos) == PrecipitationKind::Rain
    }
}

/// What `Level.precipitationAt` reads of a level, for [`precipitation_at`].
pub trait PrecipitationReads {
    /// `Level.isRaining`: the rain level is above 0.2.
    fn is_raining(&self) -> bool;
    /// `Level.canSeeSky(pos)`.
    fn can_see_sky(&self, pos: BlockPos) -> bool;
    /// `getHeightmapPos(Heightmap.Types.MOTION_BLOCKING, pos).getY()`.
    fn motion_blocking_y(&self, pos: BlockPos) -> i32;
    /// `getBiome(pos).value().getPrecipitationAt(pos, getSeaLevel())`.
    fn biome_precipitation(&self, pos: BlockPos) -> PrecipitationKind;
}

/// `Level.precipitationAt(pos)`: nothing unless it rains, the position sees the sky and nothing
/// that blocks motion is above it; then the biome's precipitation there. `Level.isRainingAt` is
/// its rain, and the server's block behaviours, entities, mobs and the mods' reads all ask it.
///
/// A native mod's `[[native.mixin]]` row can replace it, as a Java mod's mixin on
/// `precipitationAt` does: a `RETURN` row answering `PrecipitationKind::None` keeps a level dry.
#[sunrise_mod_api_macros::replaceable]
pub fn precipitation_at(level: &dyn PrecipitationReads, pos: BlockPos) -> PrecipitationKind {
    if !level.is_raining() {
        return PrecipitationKind::None;
    }
    if !level.can_see_sky(pos) {
        return PrecipitationKind::None;
    }
    if level.motion_blocking_y(pos) > pos.y {
        return PrecipitationKind::None;
    }
    level.biome_precipitation(pos)
}

/// A level's answers for [`precipitation_at`], through its block behaviours.
struct LevelPrecipitation<'a, L: LevelAccess + ?Sized> {
    b: &'a Behaviours,
    level: &'a L,
}

impl<L: LevelAccess + ?Sized> PrecipitationReads for LevelPrecipitation<'_, L> {
    fn is_raining(&self) -> bool {
        self.level.is_raining()
    }
    fn can_see_sky(&self, pos: BlockPos) -> bool {
        self.level.sky_light(pos) >= 15
    }
    fn motion_blocking_y(&self, pos: BlockPos) -> i32 {
        self.b.motion_blocking_top(self.level, pos).y
    }
    fn biome_precipitation(&self, pos: BlockPos) -> PrecipitationKind {
        match self.b.biome_settings(self.level, pos) {
            None => PrecipitationKind::None,
            Some(s) => match Behaviours::precipitation_kind(s, pos, sea_level(self.level)) {
                BiomePrecipitation::None => PrecipitationKind::None,
                BiomePrecipitation::Rain => PrecipitationKind::Rain,
                BiomePrecipitation::Snow => PrecipitationKind::Snow,
            },
        }
    }
}
