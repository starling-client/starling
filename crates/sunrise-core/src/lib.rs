//! Sunrise's initial simulation primitives, targeting the Minecraft Java Edition version in [`version`].
//!
//! This is not a playable game. Differential checks cover only the operations
//! listed in docs/rust-parity.md; the rest of vanilla remains unimplemented.
#![deny(unsafe_code)]

pub mod arm_pose;
pub mod biome;
pub mod bounding_box;
pub mod carving;
pub mod character_names;
pub mod color;
pub mod connection_kind;
pub mod crc32;
pub mod density;
pub mod dimension;
pub mod equipment_slot;
pub mod fast_hash;
pub mod game_mode;
pub mod geometry;
pub mod identifier;
pub mod java_character;
pub mod java_float_parse;
// The map's slab and table are the crate's only unsafe code; every block carries its SAFETY note.
#[allow(unsafe_code)]
pub mod java_hash_map;
pub mod java_hash_set;
pub mod java_number;
pub mod jmath;
pub mod json_nbt;
pub mod level_id;
pub mod level_key;
pub mod md5;
pub mod modified_utf8;
pub mod mth;
pub mod nbt;
pub mod neighbour_update;
pub mod no_ai;
pub mod noise;
pub mod paletted;
pub mod position;
pub mod prepared_files;
pub mod profile;
pub mod random;
pub mod release;
pub mod storage;
pub mod text;
pub mod tick_phase;
pub mod ticks;
pub mod trading;
pub mod use_animation;
pub mod uuid;
pub mod vanilla_bytes;
pub mod version;

pub use level_key::LevelKey;
pub use version::VANILLA_VERSION;

/// Queried loot table id before a root roll finishes, or for an unnamed root.
pub const UNKNOWN_LOOT_TABLE: &str = "neoforge:unknown_loot_table";
