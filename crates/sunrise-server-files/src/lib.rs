//! The dedicated server's files and settings, ported from
//! net.minecraft.server.dedicated and net.minecraft.server.players:
//! server.properties, eula.txt, the four user lists, usercache.json and
//! looking players up by name. The game's dedicated server
//! (`sunrise-client --dedicated`) reads them, and `sunrise-server` manages
//! them; this crate needs neither a window nor the game.
pub mod biome_regions;
pub mod duration;
pub mod eula;
pub mod json;
pub mod missing_mods;
pub mod private_file;
pub mod profiles;
pub mod properties;
mod seed_text;
pub mod user_cache;
pub mod user_lists;
