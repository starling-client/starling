//! Persistent data-component value codecs (122 registered names, 3 transient) and their nested records.
//! `VALUES` is the component registry table consumed by the kernel; `ROWS` holds the `component_*` helper
//! records reached through `Named`. Item templates, patches, item/block predicates and text recurse through
//! the same kernel table. Registry membership (is this ID a real mob effect, block, ...) is NOT checked here:
//! shape validation only validates identifier syntax. Unknown object fields are ignored, as in map codecs.
//!
//! Primitive constraints are interpreted by the shared kernel, not per-component validators.
use super::*;
use Kind::{
    All, Any, Bool, ComponentMap, Either, Enum, FloatRange, Id, IdMap, IntRange, List, Map, MapMax,
    Named, NonAirItemId, Number, Object, PlayerName, Snbt, String, Switch, Transient,
    TypedEntityData,
};

const MAXLEN: usize = usize::MAX;
const NNINT: Kind = IntRange(0, i32::MAX as i64);
const POSINT: Kind = IntRange(1, i32::MAX as i64);
const NNFLOAT: Kind = FloatRange(0.0, f32::MAX as f64);
const POSFLOAT: Kind = FloatRange(f32::from_bits(1) as f64, f32::MAX as f64);
const UNIT_FLOAT: Kind = FloatRange(0.0, 1.0);
// Plain INT/FLOAT/DOUBLE accept any JSON number (Number conversion); bounds are only applied where the codec has them.
const INT: Kind = Number;
const REF: Kind = Id;
const IDS: Kind = Named("ids");
const TEXT: Kind = Named("text");
const SOUND: Kind = Named("sound");
const DYE: Kind = Named("dye_color");
const TEMPLATE: Kind = Named("component_template");
const EFFECT: Kind = Named("component_effect_instance");
const CONSUME_EFFECT: Kind = Named("component_consume_effect");
const TRIM_MATERIAL: Kind = Named("component_trim_material");
const UNIT: Kind = Object(&[]);
const SWING: Kind = Named("component_swing_animation");
const fn v(name: &'static str, kind: Kind) -> Row {
    Row { name, kind }
}

// Strict optional decoding is the default; `Any` appears only where the codec is lenient (malformed -> default).
const EFFECT_DETAILS: &[Field] = &[
    opt("amplifier", IntRange(0, 255)),
    opt("duration", INT),
    opt("ambient", Bool),
    opt("show_particles", Bool),
    opt("show_icon", Bool),
    opt("hidden_effect", Named("component_effect_details")),
];
const CONSUME_EFFECTS: &[Row] = &[
    row(
        "apply_effects",
        &[
            req("effects", List(&EFFECT, 0, MAXLEN)),
            opt("probability", UNIT_FLOAT),
        ],
    ),
    row("remove_effects", &[req("effects", IDS)]),
    row("clear_all_effects", &[]),
    row(
        "teleport_randomly",
        &[
            opt("diameter", POSFLOAT),
            opt("directional_particles", Bool),
        ],
    ),
    row("play_sound", &[req("sound", SOUND)]),
];
const ATTRIBUTE_DISPLAYS: &[Row] = &[
    row("default", &[]),
    row("hidden", &[]),
    row("override", &[req("value", TEXT)]),
];
const CONDITION: Kind = Named("component_kinetic_condition");
const PROPERTY_RECORD: Kind = Object(&[
    req("name", String(64)),
    req("value", String(32767)),
    opt("signature", String(1024)),
]);

pub(super) const ROWS: &[Row] = &[
    // Registry-driven maps recurse through the same kernel table: exact component values, patches, partial predicates.
    v("component_values", ComponentMap(false)),
    v("component_patch", ComponentMap(true)),
    v("component_predicates", Kind::ComponentPredicates),
    // Item.CODEC refuses air; bare ID is allowed, unlike ItemStack. Count 1..99; components is a Patch.
    v(
        "component_template",
        Either(&[
            NonAirItemId,
            Object(&[
                req("id", NonAirItemId),
                opt("count", IntRange(1, 99)),
                opt("components", ComponentMap(true)),
            ]),
        ]),
    ),
    // UUIDUtil.CODEC: list of exactly four INTs, not a string.
    v("component_uuid", List(&Number, 4, 4)),
    v("component_resolvable_int", Either(&[Number, Id])),
    v("component_resolvable_float", Either(&[Number, Id])),
    v("component_effect_details", Object(EFFECT_DETAILS)),
    v(
        "component_effect_instance",
        All(&[Object(&[req("id", REF)]), Object(EFFECT_DETAILS)]),
    ),
    v(
        "component_consume_effect",
        Switch("type", true, CONSUME_EFFECTS),
    ),
    v(
        "component_attribute_entry",
        Object(&[
            req("type", REF),
            req("id", Id),
            req("amount", Number),
            req("operation", Named("attribute_operation")),
            opt("slot", Named("slot_group")),
            opt("display", Switch("type", false, ATTRIBUTE_DISPLAYS)),
        ]),
    ),
    row(
        "component_tool_rule",
        &[
            req("blocks", IDS),
            opt("speed", POSFLOAT),
            opt("correct_for_drops", Bool),
        ],
    ),
    v(
        "component_equipment_slot",
        Enum(&[
            "mainhand", "offhand", "feet", "legs", "chest", "head", "body", "saddle",
        ]),
    ),
    row(
        "component_equippable",
        &[
            req("slot", Named("component_equipment_slot")),
            opt("equip_sound", SOUND),
            opt("asset_id", Id),
            opt("camera_overlay", Id),
            opt("allowed_entities", IDS),
            opt("dispensable", Bool),
            opt("swappable", Bool),
            opt("damage_on_hurt", Bool),
            opt("equip_on_interact", Bool),
            opt("can_be_sheared", Bool),
            opt("shearing_sound", SOUND),
        ],
    ),
    row(
        "component_damage_reduction",
        &[
            opt("horizontal_blocking_angle", POSFLOAT),
            opt("type", IDS),
            req("base", Number),
            req("factor", Number),
        ],
    ),
    row(
        "component_item_damage_function",
        &[
            req("threshold", NNFLOAT),
            req("base", Number),
            req("factor", Number),
        ],
    ),
    row(
        "component_blocks_attacks",
        &[
            opt("block_delay_seconds", NNFLOAT),
            opt("disable_cooldown_scale", NNFLOAT),
            opt(
                "damage_reductions",
                List(&Named("component_damage_reduction"), 0, MAXLEN),
            ),
            opt("item_damage", Named("component_item_damage_function")),
            opt("bypassed_by", IDS),
            opt("block_sound", SOUND),
            opt("disabled_sound", SOUND),
        ],
    ),
    row(
        "component_kinetic_condition",
        &[
            req("max_duration_ticks", NNINT),
            opt("min_speed", Number),
            opt("min_relative_speed", Number),
        ],
    ),
    row(
        "component_kinetic_weapon",
        &[
            opt("contact_cooldown_ticks", NNINT),
            opt("delay_ticks", NNINT),
            opt("dismount_conditions", CONDITION),
            opt("knockback_conditions", CONDITION),
            opt("damage_conditions", CONDITION),
            opt("forward_movement", Number),
            opt("damage_multiplier", Number),
            opt("sound", SOUND),
            opt("hit_sound", SOUND),
        ],
    ),
    row(
        "component_swing_animation",
        &[
            opt("type", Enum(&["none", "whack", "stab"])),
            opt("duration", NNINT),
        ],
    ),
    // TrimMaterial/TrimPattern: registry reference or inline record.
    v(
        "component_trim_material",
        Either(&[
            Id,
            Object(&[req("palette_id", Id), req("description", TEXT)]),
        ]),
    ),
    v(
        "component_trim_pattern",
        Either(&[
            Id,
            Object(&[
                req("asset_id", Id),
                req("description", TEXT),
                opt("decal", Bool),
            ]),
        ]),
    ),
    // Differs from the shared `instrument` row, which lacks durability_damage (NNINT); that field must be validated here.
    v(
        "component_instrument",
        Either(&[
            Id,
            Object(&[
                req("sound_event", SOUND),
                req("use_duration", NNFLOAT),
                req("range", POSFLOAT),
                opt("durability_damage", NNINT),
                req("description", TEXT),
            ]),
        ]),
    ),
    row(
        "component_lodestone_target",
        &[req("dimension", Id), req("pos", List(&Number, 3, 3))],
    ),
    row(
        "component_map_decoration",
        &[
            req("type", REF),
            req("x", Number),
            req("z", Number),
            req("rotation", Number),
        ],
    ),
    row(
        "component_container_slot",
        &[req("slot", IntRange(0, 255)), req("item", TEMPLATE)],
    ),
    row(
        "component_banner_layer",
        &[req("pattern", Named("banner_pattern")), req("color", DYE)],
    ),
    row(
        "component_bee",
        &[
            req("entity_data", TypedEntityData),
            req("ticks_in_hive", INT),
            req("min_ticks_in_hive", INT),
        ],
    ),
    // ResolvableProfile: STRING shorthand or object. The full alternative (R id, R name) is a strict subset of the
    // partial one (all O) with identical field validation, so one object row accepts exactly their union.
    v(
        "component_profile",
        Either(&[PlayerName, Named("component_profile_record")]),
    ),
    row(
        "component_profile_record",
        &[
            opt("name", PlayerName),
            opt("id", Named("component_uuid")),
            opt("properties", Named("component_property_map")),
            opt("texture", Id),
            opt("cape", Id),
            opt("elytra", Id),
            opt("model", Enum(&["slim", "wide"])),
        ],
    ),
    // Object form: at most 16 KEYS, no record-field limits; list form: at most 16 records with field limits.
    v(
        "component_property_map",
        Either(&[
            MapMax(&List(&String(MAXLEN), 0, MAXLEN), 16),
            List(&PROPERTY_RECORD, 0, 16),
        ]),
    ),
    row(
        "component_sign_text",
        &[
            req("messages", List(&TEXT, 4, 4)),
            // optionalAlwaysPresentFieldOf with lenient decode: malformed filtered_messages (including wrong length) drops to absent.
            opt("filtered_messages", Any),
            opt("color", DYE),
            opt("has_glowing_text", Bool),
        ],
    ),
];

pub(super) const VALUES: &[Row] = &[
    v("custom_data", Snbt),
    v("max_stack_size", IntRange(1, 99)),
    v("max_damage", POSINT),
    v("damage", NNINT),
    v("unbreakable", UNIT),
    row(
        "use_effects",
        &[
            opt("can_sprint", Bool),
            opt("interact_vibrations", Bool),
            opt("speed_multiplier", UNIT_FLOAT),
        ],
    ),
    v("custom_name", TEXT),
    v("minimum_attack_charge", UNIT_FLOAT),
    v("damage_type", REF),
    v("item_name", TEXT),
    v("item_model", Id),
    v("lore", List(&TEXT, 0, 256)),
    v("rarity", Enum(&["common", "uncommon", "rare", "epic"])),
    v("enchantments", IdMap(&IntRange(1, 255))),
    v(
        "can_place_on",
        Either(&[
            Named("block_predicate"),
            List(&Named("block_predicate"), 1, MAXLEN),
        ]),
    ),
    v(
        "can_break",
        Either(&[
            Named("block_predicate"),
            List(&Named("block_predicate"), 1, MAXLEN),
        ]),
    ),
    v(
        "attribute_modifiers",
        List(&Named("component_attribute_entry"), 0, MAXLEN),
    ),
    row(
        "custom_model_data",
        &[
            opt("floats", List(&Number, 0, MAXLEN)),
            opt("flags", List(&Bool, 0, MAXLEN)),
            opt("strings", List(&String(MAXLEN), 0, MAXLEN)),
            opt("colors", List(&Named("rgb"), 0, MAXLEN)),
        ],
    ),
    // hidden_components are component-type registry IDs (transient types included); membership is a boundary.
    row(
        "tooltip_display",
        &[
            opt("hide_tooltip", Bool),
            opt("hidden_components", List(&Id, 0, MAXLEN)),
        ],
    ),
    v("repair_cost", NNINT),
    v("creative_slot_lock", Transient),
    v("enchantment_glint_override", Bool),
    v("intangible_projectile", UNIT),
    row(
        "food",
        &[
            req("nutrition", NNINT),
            req("saturation", Number),
            opt("can_always_eat", Bool),
        ],
    ),
    row(
        "consumable",
        &[
            opt("consume_seconds", NNFLOAT),
            opt(
                "animation",
                Enum(&[
                    "none",
                    "eat",
                    "drink",
                    "block",
                    "bow",
                    "trident",
                    "crossbow",
                    "spyglass",
                    "toot_horn",
                    "brush",
                    "bundle",
                    "spear",
                ]),
            ),
            opt("sound", SOUND),
            opt("has_consume_particles", Bool),
            opt("on_consume_effects", List(&CONSUME_EFFECT, 0, MAXLEN)),
        ],
    ),
    v("use_remainder", TEMPLATE),
    row(
        "use_cooldown",
        &[req("seconds", POSFLOAT), opt("cooldown_group", Id)],
    ),
    row("damage_resistant", &[req("types", IDS)]),
    row(
        "tool",
        &[
            req("rules", List(&Named("component_tool_rule"), 0, MAXLEN)),
            opt("default_mining_speed", Number),
            opt("damage_per_block", NNINT),
            opt("can_destroy_blocks_in_creative", Bool),
        ],
    ),
    row(
        "weapon",
        &[
            opt("item_damage_per_attack", NNINT),
            opt("disable_blocking_for_seconds", NNFLOAT),
        ],
    ),
    // No min/max relational check exists in the codec.
    row(
        "attack_range",
        &[
            opt("min_reach", FloatRange(0.0, 64.0)),
            opt("max_reach", FloatRange(0.0, 64.0)),
            opt("min_creative_reach", FloatRange(0.0, 64.0)),
            opt("max_creative_reach", FloatRange(0.0, 64.0)),
            opt("hitbox_margin", UNIT_FLOAT),
            opt("mob_factor", FloatRange(0.0, 2.0)),
        ],
    ),
    row("enchantable", &[req("value", POSINT)]),
    v("equippable", Named("component_equippable")),
    row("repairable", &[req("items", IDS)]),
    v("glider", UNIT),
    v("tooltip_style", Id),
    row(
        "death_protection",
        &[opt("death_effects", List(&CONSUME_EFFECT, 0, MAXLEN))],
    ),
    v("blocks_attacks", Named("component_blocks_attacks")),
    row(
        "piercing_weapon",
        &[
            opt("deals_knockback", Bool),
            opt("dismounts", Bool),
            opt("sound", SOUND),
            opt("hit_sound", SOUND),
        ],
    ),
    v("kinetic_weapon", Named("component_kinetic_weapon")),
    v("attack_animation", SWING),
    v("interact_animation", SWING),
    v("additional_trade_cost", Transient),
    v("block_transformer", REF),
    row("villager_food", &[req("nutrition", POSINT)]),
    v("stored_enchantments", IdMap(&IntRange(1, 255))),
    v("dye", DYE),
    v("dyed_color", Named("rgb")),
    // MapId: INT with no nonnegative bound.
    v("map_id", INT),
    // Keys are arbitrary STRINGs, not identifiers.
    v("map_decorations", Map(&Named("component_map_decoration"))),
    v("map_post_processing", Transient),
    v("charged_projectiles", List(&TEMPLATE, 0, 1024)),
    v("bundle_contents", List(&TEMPLATE, 0, MAXLEN)),
    v(
        "potion_contents",
        Either(&[
            Id,
            Object(&[
                opt("potion", REF),
                opt("custom_color", INT),
                opt("custom_effects", List(&EFFECT, 0, MAXLEN)),
                opt("custom_name", String(MAXLEN)),
            ]),
        ]),
    ),
    v("potion_duration_scale", NNFLOAT),
    // duration is lenient: a malformed value is accepted and defaults to 160, hence Any.
    v(
        "suspicious_stew_effects",
        List(&Object(&[req("id", REF), opt("duration", Any)]), 0, MAXLEN),
    ),
    row(
        "writable_book_content",
        &[opt("pages", List(&Named("writable_page"), 0, 100))],
    ),
    // No page-list count bound; page Text is flat-restricted to 32767 (size accounting is a boundary).
    row(
        "written_book_content",
        &[
            req("title", Named("book_title")),
            req("author", String(MAXLEN)),
            opt("generation", IntRange(0, 3)),
            opt("pages", List(&Named("written_page"), 0, MAXLEN)),
            opt("resolved", Bool),
        ],
    ),
    row(
        "trim",
        &[
            req("material", TRIM_MATERIAL),
            req("pattern", Named("component_trim_pattern")),
        ],
    ),
    // Boundary: each property name must exist in the keyed block's state definition (needs block registry).
    v("debug_stick_state", IdMap(&String(MAXLEN))),
    v("entity_data", TypedEntityData),
    v("bucket_entity_data", Snbt),
    v("block_entity_data", TypedEntityData),
    v("instrument", Named("component_instrument")),
    v("provides_trim_material", TRIM_MATERIAL),
    v("ominous_bottle_amplifier", IntRange(0, 4)),
    v("jukebox_playable", REF),
    v("provides_banner_patterns", IDS),
    v("recipes", List(&Id, 0, MAXLEN)),
    row(
        "lodestone_tracker",
        &[
            opt("target", Named("component_lodestone_target")),
            opt("tracked", Bool),
        ],
    ),
    v("firework_explosion", Named("firework_explosion")),
    // flight_duration is UNSIGNED_BYTE: Codec.BYTE input (any number), not an input 0..255 check.
    row(
        "fireworks",
        &[
            opt("flight_duration", Number),
            opt("explosions", List(&Named("firework_explosion"), 0, 256)),
        ],
    ),
    v("profile", Named("component_profile")),
    v("note_block_sound", Id),
    v(
        "banner_patterns",
        List(&Named("component_banner_layer"), 0, MAXLEN),
    ),
    v("base_color", DYE),
    row(
        "pot_decorations",
        &[
            opt("back", TEMPLATE),
            opt("left", TEMPLATE),
            opt("right", TEMPLATE),
            opt("front", TEMPLATE),
        ],
    ),
    v(
        "container",
        List(&Named("component_container_slot"), 0, 256),
    ),
    // Arbitrary STRING keys and values; no block-property lookup here.
    v("block_state", Map(&String(MAXLEN))),
    v("bees", List(&Named("component_bee"), 0, MAXLEN)),
    v("sulfur_cube_content", TEMPLATE),
    v("lock", Named("item_predicate")),
    row(
        "container_loot",
        &[req("loot_table", Id), opt("seed", Number)],
    ),
    v("break_sound", SOUND),
    row(
        "compostable",
        &[req("layers", Named("component_resolvable_int"))],
    ),
    row(
        "cooking_fuel",
        &[
            req("burn_time", Named("component_resolvable_int")),
            req("speed_multiplier", Named("component_resolvable_float")),
        ],
    ),
    row(
        "brewing_fuel",
        &[
            req("uses", Named("component_resolvable_int")),
            req("speed_multiplier", Named("component_resolvable_float")),
        ],
    ),
    row(
        "mob_visibility",
        &[
            req("targeting_entity_types", IDS),
            req("visibility", FloatRange(0.0, 10.0)),
        ],
    ),
    v("villager/variant", REF),
    v("wolf/variant", REF),
    v("wolf/sound_variant", REF),
    v("wolf/collar", DYE),
    v("fox/variant", Enum(&["red", "snow"])),
    v("salmon/size", Enum(&["small", "medium", "large"])),
    v(
        "parrot/variant",
        Enum(&["red_blue", "blue", "green", "yellow_blue", "gray"]),
    ),
    v(
        "tropical_fish/pattern",
        Enum(&[
            "kob",
            "sunstreak",
            "snooper",
            "dasher",
            "brinely",
            "spotty",
            "flopper",
            "stripey",
            "glitter",
            "blockfish",
            "betty",
            "clayfish",
        ]),
    ),
    v("tropical_fish/base_color", DYE),
    v("tropical_fish/pattern_color", DYE),
    v("mooshroom/variant", Enum(&["red", "brown"])),
    v(
        "rabbit/variant",
        Enum(&[
            "brown",
            "white",
            "black",
            "white_splotched",
            "gold",
            "salt",
            "evil",
        ]),
    ),
    v("pig/variant", REF),
    v("pig/sound_variant", REF),
    v("cow/variant", REF),
    v("cow/sound_variant", REF),
    v("chicken/variant", REF),
    v("chicken/sound_variant", REF),
    v("zombie_nautilus/variant", REF),
    v("frog/variant", REF),
    v(
        "horse/variant",
        Enum(&[
            "white",
            "creamy",
            "chestnut",
            "brown",
            "black",
            "gray",
            "dark_brown",
        ]),
    ),
    v("painting/variant", REF),
    v("llama/variant", Enum(&["creamy", "white", "brown", "gray"])),
    v(
        "axolotl/variant",
        Enum(&["lucy", "wild", "gold", "cyan", "blue"]),
    ),
    v("cat/variant", REF),
    v("cat/sound_variant", REF),
    v("cat/collar", DYE),
    v("sheep/color", DYE),
    v("shulker/color", DYE),
    v("provides_pottery_pattern", REF),
    v("sign_text_front", Named("component_sign_text")),
    v("sign_text_back", Named("component_sign_text")),
    v("waxed", UNIT),
    v("cushion/color", DYE),
];
