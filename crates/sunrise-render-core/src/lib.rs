//! Frustum geometry and terrain instance and fade helpers for Sunrise.

extern crate alloc;

pub mod frustum;
pub mod random;
pub mod stars;
pub mod terrain;

pub use frustum::{Cull, Frusta, Frustum, JoinedFrusta, MAX_WORLD_VIEWS};
pub use terrain::{
    BRIGHT, CHUNK_VISIBILITY_THRESHOLD, EntitySpan, FLAT_NORMAL, INCANDESCENT, LOW, NO_SPECULAR,
    OPAQUE_LAYER, SectionKey, TerrainSource, TerrainVertex, UV_STEPS, VERY_BRIGHT, Vertex, WHOLE,
    WeatherTime, WeatherVertex, chunk_visibility, emitting, fade_millis, section_instance,
};

pub mod entity;
pub use entity::{
    EntityVertex, FLUID_OVERLAY, HURT_TEXEL, OIT_ENTITY, PosedModelRuns, SkinRunJob,
    TERRAIN_MODEL_CUTOUT_MIPPED_OVERLAY, TERRAIN_MODEL_CUTOUT_OVERLAY, TERRAIN_MODEL_SOLID_OVERLAY,
    TERRAIN_MODEL_TRANSLUCENT_OVERLAY, TRANSLUCENT_GLINT_ITEM_OVERLAY, TRANSLUCENT_ITEM_OVERLAY,
    TRANSLUCENT_PARTICLE_OVERLAY, WATER_MASK_OVERLAY, WHITE_OVERLAY, is_native_terrain, skin_runs,
    stage_light, white_alpha,
};

pub mod frame_view;
pub use frame_view::{FogView, MAX_DYNAMIC_LIGHTS, SkyView, UniformFrameView};
