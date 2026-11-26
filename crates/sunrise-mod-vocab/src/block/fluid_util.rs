//! Fluid interaction helpers, as NeoForge 26.3's `FluidUtil`: the common
//! case of a player right-clicking a tank with a bucket or any fluid container, in one call, with
//! the bucket's sound and game event, and a container picking up and placing fluid in the world
//! as a bucket does. Vocabulary over [`FLUIDS`], [`ITEM_FLUIDS`] at an [`ItemPlace`], the move
//! step of [`move_resources`](crate::resource_util::move_resources) and the level's
//! [`pickup_fluid`](LevelRef::pickup_fluid) and [`place_fluid`](LevelRef::place_fluid), and a
//! dispenser's answer for a fluid container ([`dispense_container`]): nothing changes until a mod
//! calls them.
use crate::content::resource::{Ends, advance_transfer_index, move_once};
use crate::{
    Capability, Direction, DispenseItemEvent, Dispensed, FLUIDS, FluidResource, FluidStack,
    GameEventContext, Hand, HostBlockStates, HostTransaction, ITEM_FLUIDS, ItemPlace, LevelRef,
    PlayerRef, ResourceHandler, ScratchError, ServerHost, SoundSource, TransactionError,
    backend_mut_of,
};

/// `FluidType.BUCKET_VOLUME`: what a source block holds, and what a placement takes.
const BUCKET: u32 = 1000;

/// `FluidUtil.interactWithFluidHandler` for a block: `player`'s `hand` against the tanks of the
/// block at `pos` in `level` as `side` reaches them ([`FLUIDS`]), as
/// [`interact_with_handler`] does. False where the block has no tanks.
///
/// A tank block calls it from its `use_item_on` part and answers a success when it is true:
///
/// ```no_run
/// use mod_api::{BlockPart, Direction, HostSide, InteractionResult, UseItemOnEvent, fluid_util};
/// struct Tank;
/// #[mod_api::sunrise_part]
/// impl BlockPart for Tank {
///     fn use_item_on(
///         &self,
///         host: HostSide<'_>,
///         event: &mut UseItemOnEvent<'_>,
///     ) -> Option<InteractionResult> {
///         let HostSide::Server(host) = host else { return None };
///         let click = event.click;
///         let player = host.player(click.player)?;
///         let level = player.level(host)?;
///         let side = Direction::ALL.into_iter().find(|d| d.normal() == click.hit.face);
///         fluid_util::interact_with_fluid_handler(host, player, click.hand, level, click.pos, side)
///             .then_some(InteractionResult::Success)
///     }
/// }
/// ```
pub fn interact_with_fluid_handler(
    host: &mut dyn ServerHost,
    player: PlayerRef,
    hand: Hand,
    level: LevelRef,
    pos: [i32; 3],
    side: Option<Direction>,
) -> bool {
    if player.level(host) != Some(level) {
        return false;
    }
    let Some(handler) = level.capability(host, &FLUIDS, pos, side) else {
        return false;
    };
    interact_with_handler(host, player, hand, Some(pos), &handler)
}

/// `FluidUtil.interactWithFluidHandler` for any handler: fills the fluid container `player`
/// holds in `hand` ([`ITEM_FLUIDS`] at [`ItemPlace::PlayerHand`]) from `handler` and, when
/// nothing goes in, empties it into `handler`. Either way it moves the first fluid that moves,
/// as much of it as fits, in a transaction of its own: a stack of more than one container has
/// one split off, changed, where the place puts it, and a creative player keeps the hand's
/// stack, as [`ItemPlace::PlayerHand`] writes. Once something moved it plays the fill or empty
/// sound and game event at the centre of `pos`, or half a block above the player's feet
/// where `pos` is none, as
/// [`trigger_sound_and_game_event`]. True when something moved; false, with nothing changed,
/// where the hand holds no fluid container or nothing moves either way.
///
/// The sound and game event go out as it returns: call it outside a transaction the caller
/// may still abort.
pub fn interact_with_handler(
    host: &mut dyn ServerHost,
    player: PlayerRef,
    hand: Hand,
    pos: Option<[i32; 3]>,
    handler: &Capability<dyn ResourceHandler<FluidResource>>,
) -> bool {
    let Some(level) = player.level(host) else {
        return false;
    };
    let place = ItemPlace::PlayerHand {
        player: player.uuid(),
        hand,
    };
    let Some(item) = place.capability(host, &ITEM_FLUIDS) else {
        return false;
    };
    let moved = match move_first(host, handler, &item) {
        Ok(Some(moved)) => (moved, true),
        Ok(None) => match move_first(host, &item, handler) {
            Ok(Some(moved)) => (moved, false),
            Ok(None) | Err(()) => return false,
        },
        Err(()) => return false,
    };
    let (moved, pickup) = moved;
    let fluid = FluidResource::of(&moved);
    let at = match pos {
        Some([x, y, z]) => Some([x, y, z].map(|c| f64::from(c) + 0.5)),
        None => player
            .entity()
            .position(host)
            .map(|[x, y, z]| [x, y + 0.5, z]),
    };
    if let Some(at) = at {
        trigger_sound_and_game_event(host, &fluid, level, at, Some(player), pickup);
    }
    true
}

/// `FluidUtil.triggerSoundAndGameEvent`: what a bucket plays as `fluid` goes in or out of a
/// container at `at` in `level`. `pickup` is true when the fluid came out of the handler into the
/// container (`minecraft:fluid_pickup` and the fluid's
/// [`pickup_sound`](crate::FluidKey::pickup_sound)) and false when it went
/// in (`minecraft:fluid_place` and its
/// [`empty_sound`](crate::FluidKey::empty_sound)). The sound plays under
/// the block slider at full volume and pitch for every player near, the acting one included,
/// since the server decides the move; the game event names `player` as its source when given.
/// A fluid with no bucket sound, a tank-only one that names no bucket, has no sound, and only
/// its game event goes out.
pub fn trigger_sound_and_game_event(
    host: &mut dyn ServerHost,
    fluid: &FluidResource,
    level: LevelRef,
    at: [f64; 3],
    player: Option<PlayerRef>,
    pickup: bool,
) {
    let sound = fluid.fluid().key().and_then(|key| {
        let states = host.block_states();
        match pickup {
            true => key.pickup_sound(&states),
            false => key.empty_sound(&states),
        }
    });
    if let Some(sound) = sound {
        level.play_sound(host, sound, at, SoundSource::Blocks, 1.0, 1.0, None);
    }
    let event = match pickup {
        true => "minecraft:fluid_pickup",
        false => "minecraft:fluid_place",
    };
    let context = GameEventContext {
        source_entity: player.map(PlayerRef::entity),
        affected_state: None,
    };
    level.game_event(host, event, at, context);
}

/// `FluidUtil.tryPickupFluid` with a side: [`try_pickup_world_fluid`] first, and where the
/// block at `pos` has no fluid to give that way, one move of the first fluid its tanks give at
/// `side` ([`FLUIDS`]) into `destination`, as much as fits in one go, in a transaction of its own
/// (nested in one the caller has open, which then decides). What moved, or None with nothing
/// changed. Either way the bucket's fill sound and `minecraft:fluid_pickup` go out at the
/// block's centre at the outermost commit ([`LevelRef::play_fluid_effects`]), and not at all if
/// it aborts.
pub fn try_pickup_fluid(
    host: &mut dyn ServerHost,
    destination: &Capability<dyn ResourceHandler<FluidResource>>,
    player: Option<PlayerRef>,
    level: LevelRef,
    pos: [i32; 3],
    side: Option<Direction>,
) -> Option<FluidStack> {
    if let Some(picked) = try_pickup_world_fluid(host, destination, player, level, pos) {
        return Some(picked);
    }
    let handler = level.capability(host, &FLUIDS, pos, side)?;
    host.transaction(|host, tx| {
        let moved = move_first(host, &handler, destination)?.ok_or(())?;
        if let Some(fluid) = moved.key() {
            let _ = level.play_fluid_effects(host, pos, fluid, true, player, tx);
        }
        Ok::<_, ()>(moved)
    })
    .ok()
}

/// `FluidUtil.tryPickupFluid` without a side: the fluid source at `pos` in `level`, a
/// waterlogged block's water or a bubble column's ([`LevelRef::pickup_fluid`]), a bucket of it,
/// into `destination`, all of it or none, in a transaction of its own (nested in one the caller
/// has open, which then decides). What was picked up, or None with nothing changed: no source
/// there, or `destination` doesn't take the whole bucket. The block's updates, the bucket's
/// fill sound and `minecraft:fluid_pickup` follow at the outermost commit.
pub fn try_pickup_world_fluid(
    host: &mut dyn ServerHost,
    destination: &Capability<dyn ResourceHandler<FluidResource>>,
    player: Option<PlayerRef>,
    level: LevelRef,
    pos: [i32; 3],
) -> Option<FluidStack> {
    host.transaction(|host, tx| {
        let picked = level
            .pickup_fluid(host, pos, player, tx)
            .ok()
            .flatten()
            .ok_or(())?;
        let fluid = FluidResource::of(&picked);
        match destination
            .insert(host, tx, &fluid, picked.amount)
            .map_err(|_| ())?
            == picked.amount
        {
            true => Ok(picked),
            false => Err(()),
        }
    })
    .ok()
}

/// `FluidUtil.tryPlaceFluid` with a handler: a bucket of the first fluid of `source`, index by
/// index, that comes out whole and places at `pos` in `level` ([`LevelRef::place_fluid`]):
/// into air or a block it replaces, into a liquid container that accepts it (including nonwater
/// fluids accepted by a block part), or hissing away where water evaporates. The acceptance query
/// lends `player` when supplied. `validate` treats a container's false placement answer as failure,
/// while a normal bucket still empties (NeoForge's `validatePlaced`). Each try is a transaction
/// of its own (nested in one the caller has open, which then decides). What was placed, or
/// None with nothing changed. The block's updates, the fluid tick, a replaced block's drops
/// and the bucket's empty sound and `minecraft:fluid_place` follow at the outermost commit.
pub fn try_place_fluid(
    host: &mut dyn ServerHost,
    source: &Capability<dyn ResourceHandler<FluidResource>>,
    player: Option<PlayerRef>,
    level: LevelRef,
    pos: [i32; 3],
    validate: bool,
) -> Option<FluidStack> {
    let mut indices = 0..source.size(host);
    while let Some(index) = advance_transfer_index(source, host, &mut indices, None) {
        let Some(fluid) = source.resource(host, index) else {
            continue;
        };
        let Some(key) = fluid.fluid().key() else {
            continue;
        };
        let placed = host.transaction(|host, tx| {
            if source.extract_at(host, tx, index, &fluid, BUCKET) != Ok(BUCKET) {
                return Err(());
            }
            match level.place_fluid(host, pos, key, player, validate, tx) {
                Ok(true) => Ok(fluid.to_stack(BUCKET)),
                Ok(false) | Err(_) => Err(()),
            }
        });
        if let Ok(placed) = placed {
            return Some(placed);
        }
    }
    None
}

/// Why [`dispense_container`]'s source-level transaction did not commit its transfer.
enum Refused {
    /// The container neither picked up nor placed any fluid.
    NothingMoved,
    /// The scratch loan could not open or its stack failed write validation.
    NotRun,
    /// The remainder could not be routed, or a commit was refused.
    Remainder,
}

/// NeoForge's `DispenseFluidContainer`: a dispenser's answer for a fluid container, which a
/// container's [`dispense_item`](crate::ItemPart::dispense_item) part returns (it is no default:
/// vanilla's buckets keep their own behaviours). `event.stack` runs in a lent two-slot copy whose
/// item may change ([`HostTransaction::scratch_slots`]), reached one item at a time
/// ([`ItemPlace::OneByOne`]): the container's handler sees one container, and an empty one may
/// become a full one of another item. It picks up from the block in front first
/// ([`try_pickup_fluid`], its `FLUIDS` reached from the dispenser's side), and places a bucket
/// of what it holds there ([`try_place_fluid`], unvalidated, as a bucket) only where nothing
/// was picked up. A transfer inside an enclosing transaction is provisional until the root
/// commits; an abort publishes no effects. The dispenser's level owns the block updates, drops,
/// sounds, and game events (including a pickup through `FLUIDS`), as well as the waterlogging
/// tick and any ticks scheduled by a `place_liquid` Part through its
/// [`LevelWriteHost`](crate::LevelWriteHost).
/// Those effects publish once, at the outermost commit, after the slot result and any
/// overflow remainder are routed, and before the dispenser's click and smoke. A refused
/// remainder or commit rolls the transfer back and leaves the dispenser's stack as it was,
/// without a default dispense.
/// A single container is replaced in place (the first slot, else
/// the second when the first was emptied); when both slots remain populated, the first slot
/// stays in the dispenser slot and the second slot's remainder goes into the original dispenser,
/// never a replacement at that position. If it cannot go into that dispenser, it goes out of the
/// front with a second click (`DefaultDispenseItemBehavior.addToInventoryOrDispense`).
/// Where neither moves anything, nothing is changed and one container is dispensed as it is
/// ([`LevelRef::dispense_default`]). The detached
/// remainder retains its count until the active slot is written. Return None unchanged (no
/// override) where the dispenser's level isn't open, transactions are unavailable, or stack
/// validation fails before anything runs; do not call `dispense_default` again.
pub fn dispense_container(
    host: &mut dyn ServerHost,
    event: &DispenseItemEvent<'_>,
) -> Option<Dispensed> {
    let level = host.level(event.dimension)?;
    let [x, y, z] = event.pos;
    let [dx, dy, dz] = event.facing.normal();
    let front = [x.wrapping_add(dx), y.wrapping_add(dy), z.wrapping_add(dz)];
    let side = Some(event.facing.opposite());
    // One source-level transaction owns the transfer, the slot result and the remainder's
    // routing: a refusal anywhere rolls all three back, and nothing publishes before the last.
    let routed = host.transaction(|host, _tx| {
        let moved = host.scratch_slots(event.stack.clone(), |host, _tx, copy| {
            // The container's handler sees one container at a time, whatever the stack.
            let one = ItemPlace::OneByOne(Box::new(copy.clone()));
            let item = one.capability(host, &ITEM_FLUIDS).ok_or(())?;
            let transferred = try_pickup_fluid(host, &item, None, level, front, side).is_some()
                || try_place_fluid(host, &item, None, level, front, false).is_some();
            transferred.then_some(()).ok_or(())
        });
        match moved {
            Ok(((), (first, None))) => Ok(first),
            Ok(((), (None, Some(changed)))) => Ok(Some(changed)),
            Ok(((), (Some(first), Some(changed)))) => {
                backend_mut_of(host)
                    .ok_or(Refused::Remainder)?
                    .dispense_add_remainder(event.dimension, event.pos, event.facing, changed)
                    .map_err(|_| Refused::Remainder)?;
                Ok(Some(first))
            }
            Err(ScratchError::Aborted(())) => Err(Refused::NothingMoved),
            Err(ScratchError::Unavailable | ScratchError::Stack(_)) => Err(Refused::NotRun),
            Err(ScratchError::CommitRefused) => Err(Refused::Remainder),
        }
    });
    let left = match routed {
        Ok(left) => left,
        // Where neither moves anything, one container is dispensed as it is.
        Err(TransactionError::Aborted(Refused::NothingMoved)) => level
            .dispense_default(host, event.pos, event.facing, event.stack.clone())
            .ok()?,
        // Nothing ran, so there is no override to report.
        Err(
            TransactionError::Aborted(Refused::NotRun)
            | TransactionError::Unavailable
            | TransactionError::InvalidContext,
        ) => return None,
        // A refused remainder or commit rolled the transfer back: the dispenser keeps its
        // stack, and the default behaviour does not run (NeoForge `DispenseFluidContainer.execute`
        // falls back to it only when no fluid transferred).
        Err(TransactionError::Aborted(Refused::Remainder) | TransactionError::CommitRefused) => {
            Some(event.stack.clone())
        }
    };
    Some((left, 1000))
}

/// `FluidUtil.getFirstStackContained`: what the fluid container at `place` holds at its first
/// index that holds any ([`ITEM_FLUIDS`]), whatever the count of the stack there; it may not
/// come out, and the container may hold more. None where nothing there holds a fluid.
pub fn first_stack_contained(host: &mut dyn ServerHost, place: &ItemPlace) -> Option<FluidStack> {
    let item = place.capability(host, &ITEM_FLUIDS)?;
    let mut indices = 0..item.size(host);
    while let Some(index) = advance_transfer_index(&item, host, &mut indices, None) {
        if let Some(fluid) = item.resource(host, index) {
            let amount = item.amount(host, index);
            if amount > 0 {
                return Some(fluid.to_stack(amount));
            }
        }
    }
    None
}

/// One move of the first fluid of `from`, index by index, that `to` takes any of, as much of it
/// as `to` takes in one go (NeoForge's `ResourceHandlerUtil.moveFirst`), all or nothing in a
/// transaction of its own: what moved, or none, with nothing changed, where none moves or a
/// handler answers more than it was asked. One go, not [`move_resources`](crate::resource_util::move_resources)' repeats: a stack of
/// buckets has one filled, as vanilla's click fills one.
fn move_first(
    host: &mut dyn ServerHost,
    from: &Capability<dyn ResourceHandler<FluidResource>>,
    to: &Capability<dyn ResourceHandler<FluidResource>>,
) -> Result<Option<FluidStack>, ()> {
    let mut indices = 0..from.size(host);
    while let Some(index) = advance_transfer_index(from, host, &mut indices, None) {
        let Some(fluid) = from.resource(host, index) else {
            continue;
        };
        let want = from.amount(host, index);
        let ends = Ends {
            from,
            to,
            at: &index,
        };
        let moved = move_once(
            host,
            ends,
            want,
            |host, tx, want| from.extract_at(host, tx, index, &fluid, want),
            |host, tx, out| to.insert(host, tx, &fluid, out),
        );
        match moved {
            Ok(moved) if moved > 0 => return Ok(Some(fluid.to_stack(moved))),
            Err(
                error @ (TransactionError::InvalidContext
                | TransactionError::Unavailable
                | TransactionError::CommitRefused),
            ) => {
                log::warn!(
                    "Fluid transfer from {} to {}: {error:?}",
                    from.owner(),
                    to.owner()
                );
                return Err(());
            }
            _ => {}
        }
    }
    Ok(None)
}
