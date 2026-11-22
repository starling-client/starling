//! Client-only gameplay: what vanilla's client jar adds to the logical side.
#![forbid(unsafe_code)]
extern crate sunrise_client_lib as sunrise_client;
pub mod agent_navigation;
pub mod atlas_slots;
pub mod closest_space;
pub mod command_block_screen;
pub mod dragon_render;
pub mod entity_occlusion;
pub mod frame_autotune;
pub mod hud_gates;
pub mod input_trace;
pub mod mob_ambient;
pub mod mob_light;
pub mod motion_ids;
pub mod occlusion_culling;
pub mod particle_culling;
pub mod profile_lines;
pub mod remote;
pub mod screen_info;
pub mod section_scene;
pub mod subtitles;
pub mod tick_script;
pub mod transient_blocks;
pub mod tutorial;
pub mod upload_pace;
