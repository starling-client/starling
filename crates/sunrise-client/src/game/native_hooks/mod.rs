//! Native hook implementations, split by catalog area.
//!
//! Hook documentation and contract versions live in the catalog, not a shared changelog.
//!
//! Native mods are built against an exact game revision, rather than a hook API revision.
//! Bodies have no `cfg` and no macros: horizon-mixins refuses a target with
//! either (`tests::hook_points_stay_hookable`).

/// Generated dispatch entry points, not hand-written hook implementations.
mod fire;
pub(in crate::game) use block_entity_client_tick::ResidentReadBlockEntity;
pub(in crate::game) use block_entity_tick::ResidentBlockEntityAccess;
pub(in crate::game) use build_creative_mode_tab_contents::CreativeCache;
pub(in crate::game) use client_pick::ClientPick;
pub(in crate::game) use fire::{
    ConnectsToCall, EntityChangeBlockPreCall, HotbarAttackIndicatorProgressCall,
    ItemAfterVerticalCollisionCall, PlayerPongCall, UpdateShapeCall,
};
use mod_api::{BlockEntityDataSource, BlockEntityRef, ClockAdjustment};

use super::native_data_maps::DataMapType;
use super::{
    Game, GameLevel, LevelDimension, Screen,
    execution_role::{self, ExecutionRole},
    mod_api_host,
    native_block_entity_draw::BlockEntityDraw,
    native_mob_draw::{MobRender, PlayerRender},
    native_panel_recipes::RecipePage,
};
use mod_api::__private::game::{
    BlockStateDefinitionRegistration, ItemModelRegistration, ItemTintSourceRegistration,
    JsonAnimationTypeRegistration, LayerDefinitionRegistration, ModelLoaderRegistration,
    ParticleGroupRegistration, PresetEditorFactoryRegistration, RangeSelectPropertyRegistration,
    SelectPropertyRegistration, SpriteSourceRegistration, StandaloneModelRegistration,
};
use mod_api::GameMode;
use mod_api::GameTests;
pub use mod_api::game::{PhantomSpawnResult, SleepProblem};
use mod_api::{
    self, Answered, BlockPlacement, BlockTintSources, CauldronFluidContents, CauldronInteractions,
    ClientAudioOptions, ClientChatOptions, ClientGraphicsOptions, ClientGuiOptions,
    ClientInputOptions, ClientMiningPrediction, ClientModifiers, ClientReloadListeners,
    ColorResolvers, EnchantmentEffects, EnchantmentInfo, EntityLayers, EntityRef, EntityRenderers,
    ExperienceOrbRef, GuiLayerRegistrar, HandDrawContext, HandRender, InsideEffects,
    InteractionAttempt, InteractionKey, ItemEntityRef, LifecycleContext, LivingRef, LoadConditions,
    NoteBlockChange, ParticleProviders, PathComputationType, Placement, PlayerRef,
    PoiTypeExtensions, RecipeBookCategory, ScreenId, ScreenOpening, ScreenTarget, ScreenWidgets,
    ScreenshotImage, ServerReloadListeners, ServerTickPhase, ShaderPackState,
    SpecialModelRenderers, ToolUse, VillagerProfessions,
};
use mod_api::{
    ActiveEffect, AttachmentStore, BlockClick, BlockUpdateFlags, BonemealSource, ChunkRef,
    FluidStack, ItemClick, LevelRef, LogicalSide, NeighbourUpdate, TooltipElement, TriState,
    UsePhase,
};
use mod_api::{
    BlockKey, DebugEntry, DebugRenderer, EffectKey, FrameModel, FrameModelEdit, MapDecorationType,
    NameTagLine, SkullModel, TextureAtlasRegistration,
};
pub(crate) use mod_api::{
    BlockRim, ClientPacket, ClientPayload, ColoredLight, ContainerLootFilled, ExplosionStart,
    ItemFished, LootDrops, PanelEvent, PortalShape, ScreenView,
};
use mod_api::{CameraType, Despawn, Identifier, MobRef, PermissionLevel, RailShape};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use sunrise_client::block_contact::Pending;
#[cfg(test)]
use sunrise_client::mobs::Damage;
use sunrise_client::{
    atlas_sources::qualified,
    biome::{BiomeColors, SpawnEntry},
    block_entities::Pos,
    containers::{
        BlockEntity, MOD_MENUS, ModBlockEntityFields, ModContainer, ModEnergyBuffer, ModFluidTanks,
    },
    crafting::{Ingredient, recipe_result},
    extras::Corner,
    fast_hash::FxHashMap,
    first_person::Hand,
    health::{
        Attack, DamageContainer, DamageHooks, DamageLevel, HurtSource, Hurting, Knock, ShieldBlock,
        Victim,
    },
    hud::{HudBatch, rgba},
    items::ItemStack,
    keybinds,
    merchant::MerchantOffer,
    mobs::Who,
    mod_list::{self, Installed, KeyMapping, NativePage, NativeSetting, NativeSettingValue},
    music::{Music, MusicLayer},
    particles::{ParticleOptions, ParticleType},
    text::Text,
};
use sunrise_client_core::gamerules::Value as GameRuleValue;
use sunrise_client_core::suggestion::Suggestion;
use sunrise_core::{
    connection_kind::ConnectionKind,
    geometry::Direction,
    position::BlockPos,
    ticks::{ScheduleResult, TickPriority},
};
use sunrise_world::level::{BonemealCheck, PlaceContext};
use winit::{event::MouseButton, keyboard::KeyCode};
use {
    sunrise_core::{
        nbt::{Compound, Tag},
        use_animation::UseAnimation,
    },
    sunrise_world::{
        fluid::FluidKind,
        level::{LevelAccess, PlacementContext, Ticks},
        registry::{BlockRegistry, BlockStateId},
    },
};

mod add_attribute_tooltips;
mod add_client_reload_listeners;
mod add_pack_finders;
mod add_section_geometry;
mod add_server_reload_listeners;
mod adjacent_block_path_type;
mod affect_neighbors_after_removal;
mod animal_tame;
mod animate_tick;
#[cfg(test)]
mod animate_tick_tests;
mod anvil_update;
mod apply_enchantments;
mod arm_pose_transform;
mod armor_hurt;
mod arrow_nock;
mod baby_entity_spawn;
mod base;
mod beacon_color_multiplier;
mod block_drops;
mod block_entity_client_tick;
#[cfg(test)]
mod block_entity_client_tick_tests;
mod block_entity_data_received;
mod block_entity_load_additional;
mod block_entity_neighbors;
#[cfg(test)]
mod block_entity_neighbors_tests;
mod block_entity_on_load;
mod block_entity_on_load_routes_tests;
mod block_entity_on_load_tests;
mod block_entity_save_additional;
mod block_entity_save_additional_tests;
mod block_entity_tick;
mod block_entity_type_add_blocks;
mod block_entity_view_distance;
mod block_grow_feature;
mod block_landing;
mod block_on_place;
mod block_particle_effects;
#[cfg(test)]
mod block_particle_effects_tests;
mod block_path_type;
mod block_render_visibility;
mod block_sticky;
mod block_tool_modification;
mod block_use_item_on;
mod block_use_without_item;
mod bonemeal;
mod bounce_restitution;
mod break_progress_positions;
mod build_creative_mode_tab_contents;
mod calculate_player_turn;
mod camera_position;
mod camera_read;
mod camera_submersion;
mod camera_type;
mod can_be_connected_to;
#[cfg(test)]
mod can_be_connected_to_tests;
mod can_be_hydrated;
mod can_continue_sleeping;
mod can_continue_using;
mod can_drop_from_explosion;
mod can_entity_destroy;
mod can_equip;
mod can_harvest_block;
mod can_perform_action;
mod can_place_item;
mod can_player_sleep;
mod can_start_swimming;
mod can_sustain_plant;
mod chat_recipient;
mod chat_suggestion_applied;
mod chunk_ticket_level_updated;
mod chunk_watch;
mod client_attack_target;
mod client_block_changed;
mod client_break_progress;
mod client_chat;
mod client_chat_received;
#[cfg(test)]
mod client_chunk_hooks_tests;
mod client_chunk_load;
mod client_chunk_unload;
mod client_gui_state;
mod client_information_updated;
mod client_input_options;
mod client_input_reads;
mod client_input_reads_tests;
mod client_mining_predict;
mod client_narrator;
#[cfg(test)]
mod client_narrator_tests;
mod client_options;
mod client_packet;
mod client_packet_send;
mod client_particle_add;
mod client_pause_change;
mod client_pause_read;
mod client_pick;
mod client_player_change_game_type;
mod client_player_clone;
mod client_player_logging_in;
mod client_player_logging_out;
mod client_services;
mod client_setup;
mod client_stopped;
mod client_stopping;
mod client_take_item_entity;
#[cfg(test)]
mod client_take_item_entity_tests;
mod client_text;
mod client_tick_post;
mod client_tick_pre;
mod client_visual_text;
mod clone_item_stack;
mod component_hover;
mod compute_camera_angles;
mod compute_fov_modifier;
mod connects_to;
mod console_line;
mod contained_fluid;
mod container_loot_filled;
mod contents_changed;
mod crafting_remainder;
mod create_fluid_source;
mod create_skull_models;
mod create_spawn_position;
mod creative_tabs_read;
#[cfg(test)]
mod creative_tabs_read_tests;
mod creator_mod_id;
mod crop_grow;
mod custom_click_action;
mod custom_spawners;
mod customize_boss_event_progress;
mod customize_chat_overlay;
mod damage_item;
mod data_maps_updated;
#[cfg(test)]
mod death_source_tests;
mod default_components_bound;
mod destroy_progress;
mod dispense_method;
mod dragon_fight;
mod draw_measures;
mod effect_particle_modification;
mod empty_contents;
mod enchant_power_bonus;
mod enchanted_block_loot;
mod enchanted_entity_loot;
#[cfg(test)]
mod enchantment_and_lang_tests;
mod enchantment_level_set;
mod enchantments;
mod enderman_anger;
mod entity_change_block_pre;
mod entity_constructing;
mod entity_entering_section;
mod entity_inside;
mod entity_interact_specific;
mod entity_invulnerability_check;
mod entity_mob_griefing;
mod entity_outline;
mod entity_place;
mod entity_size;
mod entity_struck_by_lightning;
mod entity_travel_to_dimension;
mod exp_drop;
mod explosion_detonate;
mod explosion_knockback;
mod explosion_notice_containment;
mod explosion_resistance;
mod explosion_start;
mod extend_poi_types;
mod extract_block_screen_effect;
mod falling_tick;
mod farmland_trample;
mod fence_gate_in_wall;
#[cfg(test)]
mod fence_gate_in_wall_tests;
mod finalize_spawn;
mod fire_behaviour;
mod fire_odds;
mod fishing_medium;
mod flip_frame;
mod fluid_draw;
mod fluid_passage;
mod fluid_place_block;
mod fluid_rules;
mod fluid_tick;
mod fluid_tooltip;
mod frame_graph_setup;
#[cfg(test)]
mod framerate_limit_tests;
mod friction;
mod game_rule_changed;
mod game_shutting_down;
mod gather_effect_screen_tooltips;
mod gather_skipped_attribute_tooltips;
mod gaze_disguise;
mod general_entity_tick;
mod get_arm_pose;
mod grindstone_place;
mod grindstone_take;
#[cfg(test)]
mod guest_death_lifecycle_tests;
mod gui_status_heights;
mod gui_ticks;
mod held_item_tint;
mod highlight_tip;
mod hotbar_attack_indicator_progress;
mod hud_mode;
mod humanoid_setup_anim;
mod ignited_by_lava;
mod incoming_tests;
mod ingredient_test;
mod input_key;
mod interaction;
mod is_fertile;
mod is_fire_source;
mod is_not_replaceable_by_pick_action;
mod is_pathfindable;
mod is_piglin_currency;
mod is_possible_to_respawn_in;
mod is_primary_item_for;
mod is_scaffolding;
pub(crate) use is_scaffolding::is_scaffolding;
mod is_shearable;
mod is_valid_book_shelf;
mod item_after_vertical_collision;
mod item_attack;
mod item_container;
mod item_crafted;
mod item_destroyed;
mod item_fished;
mod item_interact_mob;
mod item_model_property;
mod item_picked_up;
#[cfg(test)]
mod item_picked_up_tests;
mod item_smelted;
mod item_stacked_on_other;
mod item_tooltip;
mod item_toss;
mod key_mapping_pre;
mod left_click_empty;
mod level_save;
mod level_sounds;
mod level_tick_tests;
mod level_world;
mod light_emission;
mod living_breathe;
mod living_change_target;
mod living_conversion_post;
mod living_conversion_pre;
mod living_damage_post;
mod living_damage_pre;
mod living_death;
mod living_death_message;
mod living_death_post;
mod living_death_protection;
mod living_destroy_block;
mod living_drops;
mod living_experience_drop;
mod living_get_projectile;
mod living_shield_block;
mod living_use_totem;
mod living_visibility;
mod makes_piglins_neutral;
mod map_color;
mod map_decoration_types;
mod math;
mod menu_extra_data;
mod menu_links;
mod menu_pick_all;
mod menu_removed;
mod menu_slot_changed;
mod menu_slot_rules;
mod menu_slot_take;
mod menu_transfer;
mod minecart_pass;
mod mob_effect_events;
mod mob_spawn_checks;
mod mob_split;
#[cfg(test)]
mod mod_block_entity_payload_tests;
mod mod_mismatch;
pub(in crate::game) mod mod_settings;
mod model_baking_completed;
mod movement_input;
mod muffle_sounds;
mod must_survive;
mod native_note_block_change;
mod native_potential_spawns;
mod native_village_siege;
mod neighbor_notify;
mod note_block_play;
mod on_block_exploded;
mod on_block_placed_by;
#[cfg(test)]
mod on_block_placed_by_tests;
mod on_block_state_change;
#[cfg(test)]
mod on_block_state_change_tests;
mod on_caught_fire;
mod on_chunk_unloaded;
#[cfg(test)]
mod on_chunk_unloaded_tests;
mod on_destroyed_by_player;
mod on_destroyed_by_push_reaction;
#[cfg(test)]
mod on_destroyed_by_push_reaction_tests;
mod on_dropped_by_player;
mod on_entity_item_update;
mod on_item_use_first;
mod on_sheared;
mod on_stop_using;
mod panel_recipes;
#[cfg(test)]
mod panel_recipes_tests;
mod permissions_changed;
mod piston_carries_block_entity;
mod piston_push_reaction;
#[cfg(test)]
mod piston_push_reaction_tests;
mod place_liquid;
mod placement_context;
#[cfg(test)]
mod placement_context_tests;
mod play_break_sound;
#[cfg(test)]
mod play_break_sound_tests;
mod play_hit_sound;
#[cfg(test)]
mod play_hit_sound_tests;
pub(in crate::game) mod play_sound;
mod player_bed_leave;
mod player_break_speed;
mod player_brewed_potion;
mod player_destroy_item;
mod player_destroyed_block;
mod player_enchant_item;
mod player_flyable_fall;
mod player_game_mode_changed;
mod player_harvest_check;
mod player_heart_type;
mod player_inventory_changed;
mod player_item_smithed;
mod player_leave;
mod player_move;
mod player_passage_cross;
mod player_pong;
mod player_set_spawn;
mod player_spawn_phantoms;
mod player_start_spectating_entity;
mod player_swap_hands;
mod player_turn;
mod player_xp_change;
mod player_xp_level_change;
mod portal_spawn;
mod potion_brew_post;
mod potion_brew_pre;
mod pre_remove_side_effects;
mod prepare_render_buffers;
mod preset_title;
mod rails;
mod recipe_book_categories;
mod recipe_book_ghost;
mod recipe_book_place;
mod recipe_sources;
mod register_block_state_models;
mod register_block_tint_sources;
mod register_cauldron_fluid_content;
mod register_cauldron_interaction;
mod register_client_item_extensions;
mod register_client_tooltip_component_factories;
mod register_color_resolvers;
mod register_configuration_tasks;
mod register_debug_entries;
mod register_debug_renderers;
mod register_dimension_transition_screen;
mod register_enchantment_effects;
mod register_entity_layers;
mod register_entity_renderers;
mod register_entity_spectator_shaders;
mod register_fluid_models;
mod register_game_tests;
mod register_gui_layers;
mod register_item_models;
mod register_item_tint_sources;
mod register_json_animation_types;
mod register_layer_definitions;
mod register_load_conditions;
mod register_mob_parts;
mod register_model_loaders;
mod register_particle_groups;
mod register_particle_providers;
mod register_pipeline_modifiers;
mod register_preset_editors;
mod register_range_select_item_model_property;
mod register_render_pipelines;
mod register_select_item_model_property;
mod register_special_model_renderer;
mod register_sprite_sources;
mod register_texture_atlases;
mod register_villager_professions;
mod registered_potions;
#[cfg(test)]
mod registered_potions_tests;
mod remove_block;
#[cfg(test)]
mod remove_block_tests;
mod render_arm;
mod render_frame_post;
mod render_frame_pre;
mod render_gui_post;
// Starling: Horizon's hook point for mixin mods (gui_layer_draw.rs).
mod gui_layer_draw;
mod render_gui_pre;
mod render_hand;
mod render_item_in_frame;
mod render_level_stage;
mod render_living_post;
mod render_living_pre;
mod render_name_tag_can_render;
mod render_name_tag_do_render;
#[cfg(test)]
mod render_name_tag_do_render_tests;
mod render_player_post;
mod render_player_pre;
mod render_tooltip_gather_components;
mod render_tooltip_pre;
pub(in crate::game) mod render_tooltip_texture;
mod render_view_transform;
pub(in crate::game) mod resolve_player_skin;
mod respawn_position;
mod right_click_empty;
mod right_click_item;
mod runtime_environment;
mod screen_chars;
mod screen_closing;
mod screen_exclusion_areas;
mod screen_hovered_stack;
mod screen_init_post;
mod screen_input_post;
#[cfg(test)]
mod screen_input_tests;
mod screen_opening;
mod screen_preedit_effects;
mod screen_render_stages;
mod screen_room;
#[cfg(test)]
mod screen_slot_tests;
mod screen_transparent_background;
mod screenshot;
mod section_visibility;
mod server_chat;
pub(in crate::game) mod server_level_event;
#[cfg(test)]
mod server_level_event_tests;
mod server_tick_rate_changed;
mod set_placed_by;
mod should_cause_block_break_reset;
mod should_cause_reequip_animation;
mod should_check_weak_power;
mod should_display_fluid_overlay;
mod should_hide_adjacent_fluid_face;
mod should_redstone_wire_connect_to;
mod sleep_status_announce;
mod sleepers_needed;
mod sound_and_shader_helpers;
mod sound_type;
mod spawn_cluster_size;
mod spawner_active;
mod spread_players_teleport;
mod state_at_viewpoint;
mod stateful_block_entity;
mod submit_custom_geometry;
mod substitute_inline_images;
mod supports_enchantment;
mod survival_query_guard;
mod sweep_attack;
mod tags_updated;
mod teleport_command;
mod texture_atlas_stitched;
mod tick_target_millis;
mod tick_weather_effects;
#[cfg(test)]
mod tick_weather_effects_tests;
mod toast_add;
mod tool_modified_state;
mod translate;
#[cfg(test)]
mod trident_family_tests;
mod use_item_hand;
mod use_item_on_block;
mod vanilla_game_event;
mod viewed_level_sound;
mod wind_burst;
mod window_resize;
mod xp_orb_targeting;
mod xp_repair_ratio;
pub(in crate::game) use add_attribute_tooltips::HookedAttributeTooltips;
pub(in crate::game) use base::LevelView;
pub(in crate::game) use bounce_restitution::install_block_contact_hooks;
pub(in crate::game) use living_change_target::NativeHookHost;
use mod_api::ModBlockEntity;
use sound_and_shader_helpers::finite_params;
pub(super) use sound_and_shader_helpers::sound_id;
pub(in crate::game) use survival_query_guard::enter_query as enter_survival_query;
include!("shared.rs");
