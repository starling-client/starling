//! Client-only surfaces: screens, input, sound, drawing and options.

pub(crate) mod audio_stream;
pub(crate) mod block_extra_models;
pub(crate) mod block_state_definition;
pub(crate) mod client_executor;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod camera;
pub(crate) mod client_reload;
pub(crate) mod client_session;
pub(crate) mod debug_entries;
pub(crate) mod dynamic_texture;
pub(crate) mod entity_layers;
pub(crate) mod entity_render_attachments;
pub(crate) mod entity_renderers;
pub(crate) mod item_extensions;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod extras;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod extras_hud;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod first_person;
pub(crate) mod fluid_draw;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod fog;
pub(crate) mod gui_clip;
pub(crate) mod gui_entity;
pub(crate) mod gui_layers;
pub(crate) mod gui_model;
pub(crate) mod gui_paint;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod hud_layout;
// hidden: module of API-owned HUD values; mods use the re-export
#[doc(hidden)]
pub mod hud_mode;
pub(crate) mod key_mapping;
pub(crate) mod keyframe_animation;
pub(crate) mod menu_screen;
pub(crate) mod mod_key_refusal;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod mesher;
pub(crate) mod microphone;
pub(crate) mod model_instance;
pub(crate) mod model_layers;
pub(crate) mod model_loader;
pub(crate) mod movement_input;
// #861 part 1: keys and views of the baked model cache.
pub(crate) mod baked_models;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod music;
pub(crate) mod narration;
pub(crate) mod pack_finders;
pub(crate) mod particle_keys;
pub(crate) mod particle_providers;
mod particle_stacks;
pub(crate) mod particle_types;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod particles;
// hidden: a module of game-side types; mods use their re-exports
#[doc(hidden)]
pub mod player_chat;
pub(crate) mod player_skin;
pub(crate) mod posed_parts;
pub(crate) mod recipe_page;
pub(crate) mod render_pipeline;
pub(crate) mod scene;
pub(crate) mod screen_slot_refusal;
pub(crate) mod screenshot;
pub(crate) mod section_visibility;
pub(crate) mod skull_models;
pub(crate) mod special_model_renderers;
pub(crate) mod standalone_models;
pub(crate) mod terrain_draw;
// hidden: module of API-owned label layout values; mods use the re-export
#[doc(hidden)]
pub mod text_layout;
pub(crate) mod types;
/// Buttons and text fields for mod screens.
pub mod widgets;
pub(crate) mod world_draw;

pub(crate) mod spectator_shader;

pub(crate) mod arm_pose;
