//! Predicates behind a client answering a right click on a remote mob.
use sunrise_client::mobs::{Mob, Species, equine, llama, panda};

/// `AbstractNautilus.mobInteract` on a client: its food is answered while the
/// nautilus takes it (taming items for an untamed adult, its food once tamed),
/// and a tame adult answers every click because that is the mount.
pub fn nautilus_takes(food: bool, tame: bool, baby: bool) -> bool {
    food || (tame && !baby)
}

/// `IronGolem.mobInteract`: an iron ingot heals 25 and only answers SUCCESS when
/// that changed the health, so a full golem leaves the click to the item.
pub fn iron_golem_takes(item: &str, health: f32, max_health: f32) -> bool {
    item == "minecraft:iron_ingot" && health < max_health
}

/// `CopperGolem.mobInteract` on a client: an empty hand makes the golem throw
/// what it holds, and shears answer only when `readyForShearing`. Honeycomb and
/// axes return `PASS` here because the waxing and scraping run on the server.
pub fn copper_golem_takes(item: &str, shears: bool, holding: bool, shearable: bool) -> bool {
    (item.is_empty() && holding) || (shears && shearable)
}

/// `AbstractHorse.handleEating` (and the llama and camel variants): this
/// mouthful is the one that ages the baby up, which is the same moment the
/// level draws one happy villager particle.
///
/// `handleEating` gates both on `isBaby() && !isAgeLocked()` — a locked baby
/// does not grow, and the particle goes with the growth.
pub fn ages_on_food(m: &Mob, item: &str) -> bool {
    if m.extra.age_locked || item.is_empty() {
        return false;
    }
    // Tadpole.feed draws the same particle on both sides; a tadpole is never
    // `isBaby`, its age counts up to the frog.
    if m.species() == Species::Tadpole {
        return m.kind.food.iter().any(|f| f.as_str() == item);
    }
    if !m.is_baby() {
        return false;
    }
    match m.species() {
        // Llama.handleEating: wheat 10, hay 90.
        s if llama::is_llama(s) => matches!(item, "minecraft:wheat" | "minecraft:hay_block"),
        // Camel.handleEating: its food ages it by 10.
        Species::Camel => m.kind.food.iter().any(|f| f.as_str() == item),
        // AbstractHorse.handleEating: every food but the red mushroom.
        Species::Horse | Species::Donkey | Species::Mule => matches!(
            item,
            "minecraft:wheat"
                | "minecraft:sugar"
                | "minecraft:hay_block"
                | "minecraft:apple"
                | "minecraft:carrot"
                | "minecraft:golden_carrot"
                | "minecraft:golden_apple"
                | "minecraft:enchanted_golden_apple"
        ),
        _ => false,
    }
}

/// `Game::remote_mob_swings` for mob `m` and the held `item`.
pub fn mob_swings(m: &Mob, item: &str) -> bool {
    let food = m.kind.food.iter().any(|f| f.as_str() == item);
    let hurt = m.health < m.attrs.max_health;
    match m.species() {
        // AbstractHorse.fedFood (Camel's, Llama's too): SUCCESS_SERVER,
        // which swings on the server's side only.
        s if food
            && (equine::is_equine(s)
                || llama::is_llama(s)
                || matches!(s, Species::Camel | Species::CamelHusk)) =>
        {
            return false;
        }
        // Panda.mobInteract: getting up off its back is SUCCESS, and its
        // bamboo answers SUCCESS_SERVER.
        Species::Panda if panda::is_on_back(m) => return true,
        Species::Panda if food => return false,
        // WanderingTrader.mobInteract: trading answers SUCCESS_SERVER, and so
        // does a spawn egg (Mob.checkAndHandleImportantInteractions). A lead is
        // CONSUME on the client (Entity.interact: a trader can't be leashed), so
        // nothing swings; a named name tag is decided by the caller.
        Species::WanderingTrader if item != "minecraft:name_tag" => return false,
        // Cat.mobInteract: food tames an untamed cat, and heals a hurt
        // tame one: SUCCESS before Animal.mobInteract is reached.
        Species::Cat if food && (!m.extra.tame || hurt) => return true,
        // Wolf.mobInteract: food heals a hurt tame wolf (SUCCESS).
        Species::Wolf if food && m.extra.tame && hurt => return true,
        _ => {}
    }
    // Animal.mobInteract: a baby's food is SUCCESS, since it ages up; an
    // adult's is CONSUME, as only the server knows whether it can breed.
    if m.kind.flags.animal
        && !matches!(m.species(), Species::Nautilus | Species::ZombieNautilus)
        && food
    {
        return m.is_baby() && !m.extra.age_locked;
    }
    // A tame wolf, cat or parrot sits or stands up: SUCCESS
    // (TamableAnimal's subclasses), so it swings.
    let consumes = match m.species() {
        // ZombieVillager.mobInteract: a golden apple cures it on the server.
        Species::ZombieVillager => item == "minecraft:golden_apple",
        // Sheep.mobInteract: shears, sheared or not.
        Species::Sheep => item == "minecraft:shears",
        _ => false,
    };
    !consumes
}
