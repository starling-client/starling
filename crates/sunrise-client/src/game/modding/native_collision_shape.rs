//! The `collision_shape` Part method (vanilla's `getCollisionShape` with its `CollisionContext`).
//! The client library asks through `native_client_hooks` from inside a move; only a state whose
//! block has a subscribing part is converted, and a part that faults is switched off.
use super::{
    Game,
    api_values::{TryIntoApi, block_state},
};
use std::{cell::RefCell, collections::HashSet, rc::Rc};
use sunrise_core::position::BlockPos;
use sunrise_world::{native_load_context::LoopContext, registry::BlockStateId};

thread_local! {
    /// The parts of the level being ticked, kept between ticks so their answers' storage and the
    /// names they share are not built again each tick.
    static PARTS: RefCell<Option<Rc<sunrise_client::collision_parts::ModParts>>> =
        const { RefCell::new(None) };
    /// The blocks whose `collision_shape` parts faulted, for the session: in memory, never
    /// saved, and a new load of the mod set starts empty.
    static DISABLED: RefCell<HashSet<mod_api::BlockKey>> = RefCell::new(HashSet::new());
}

/// The game's side of `sunrise_client::collision_parts::native_collision_shape`. A panic in a
/// part, or an answer over the shape bounds, keeps the block's normal shape, switches the
/// block's `collision_shape` parts off for the session and is logged once, naming the mods and
/// the block.
pub(crate) fn collision_shape(
    context: &LoopContext,
    pos: BlockPos,
    state: BlockStateId,
    query: &mod_api::CollisionQuery,
    shape: &mut Option<Vec<[f64; 6]>>,
) {
    if !block_state::block_listens(mod_api::HookId::CollisionShape, state) {
        return;
    }
    let Ok(api_state) = state.try_into_api() else {
        return;
    };
    let block = api_state.block();
    if DISABLED.with_borrow(|disabled| disabled.contains(&block)) {
        return;
    }
    let mut boxes = None;
    let ran = sunrise_client::crash::contain(|| {
        mod_api::dispatch_collision_shape(
            &sunrise_client_render::api_context::LoopReader(context).context(),
            pos,
            api_state,
            query,
            &mut boxes,
        );
    });
    match ran {
        Ok(()) => *shape = boxes,
        Err(error) => disable(state, &format!("panicked: {error}")),
    }
}

/// A part answered a shape the bounds refuse: the block keeps its normal shape.
pub(crate) fn refused(state: BlockStateId, error: &str) {
    disable(
        state,
        &format!("answered a shape that was refused: {error}"),
    );
}

/// Switches the block of `state` off for the session and logs it once, naming the mods and the
/// block.
fn disable(state: BlockStateId, fault: &str) {
    let Ok(api_state) = state.try_into_api() else {
        return;
    };
    let block = api_state.block();
    if DISABLED.with_borrow_mut(|disabled| disabled.insert(block)) {
        use mod_api::BlockMetadata;
        let name = mod_api::__private::block_definitions()
            .and_then(|blocks| blocks.name(block).ok().map(str::to_owned))
            .unwrap_or_else(|| format!("block {}", state.0));
        log::error!(
            "collision_shape of {name} (mods {}) {fault}; it is disabled for the session and the \
             block keeps its normal shape",
            mod_api::__private::block_key_owners(mod_api::HookId::CollisionShape, block).join(", ")
        );
    }
}

impl Game<'_> {
    /// Marks the states whose parts answer `collision_shape` in the collision tables, so scans
    /// keep them in the cells around their own. Between ticks, after the parts are resolved.
    pub(in crate::game) fn mark_collision_shape_states(&self) {
        let registry = self.collision.registry();
        self.collision.set_part_states(
            (0..registry.len())
                .map(|id| BlockStateId(id as u32))
                .filter(|state| {
                    block_state::block_listens(mod_api::HookId::CollisionShape, *state)
                }),
        );
        DISABLED.with_borrow_mut(HashSet::clear);
    }

    /// The parts that answer this tick's collision queries, until the guard drops: none when no
    /// mod has a part that answers them, so the tick pays nothing.
    pub(in crate::game) fn collision_parts(
        &self,
    ) -> Option<sunrise_world::part_collision::InParts> {
        mod_api::__private::listened(mod_api::HookId::CollisionShape).then(|| {
            let parts = PARTS.with_borrow_mut(|slot| {
                let parts = slot.get_or_insert_with(|| {
                    Rc::new(sunrise_client::collision_parts::ModParts::new(
                        self.loop_context.clone(),
                    ))
                });
                parts.begin_tick(&self.loop_context);
                Rc::clone(parts)
            });
            sunrise_world::part_collision::InParts::enter(parts)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

    #[derive(Clone, Copy)]
    enum Mode {
        Answer,
        Panic,
        Refuse,
    }
    struct Part(Mode, Arc<AtomicUsize>);
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for Part {
        fn collision_shape(
            &self,
            _: &dyn mod_api::LoopContext,
            event: &mut mod_api::CollisionShapeEvent<'_>,
        ) {
            self.1.fetch_add(1, Ordering::SeqCst);
            match self.0 {
                Mode::Answer => *event.shape = Some(vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]),
                Mode::Panic => panic!("a part that faults"),
                Mode::Refuse => *event.shape = Some(vec![[f64::NAN; 6]]),
            }
        }
    }

    /// A part that answers, one that panics and one whose answer is over the bounds, on stone,
    /// dirt and gravel: a faulting block keeps its normal shape and is not asked again.
    #[test]
    fn a_faulting_part_is_switched_off_for_the_session_and_the_others_keep_answering() {
        let calls: Vec<_> = (0..3).map(|_| Arc::new(AtomicUsize::new(0))).collect();
        struct Listener(Vec<Arc<AtomicUsize>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Listener {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                for (block, mode, calls) in [
                    ("minecraft:stone", Mode::Answer, &self.0[0]),
                    ("minecraft:dirt", Mode::Panic, &self.0[1]),
                    ("minecraft:gravel", Mode::Refuse, &self.0[2]),
                ] {
                    let key = registrar.block_key(block);
                    registrar.block_part(key, Part(mode, calls.clone()));
                }
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "collision_fault",
            Box::new(Listener(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let facts = r#"{"defaults":{"minecraft:air":3,"minecraft:stone":0,"minecraft:dirt":1,"minecraft:gravel":2},"states":[["s",false,false,"","minecraft:stone"],["d",false,false,"","minecraft:dirt"],["g",false,false,"","minecraft:gravel"],["a",true,false,"","minecraft:air"]]}"#;
            let registry =
                sunrise_world::registry::BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
            crate::game::api_values::resolve_block_definitions(&registry).unwrap();
            mod_api::__private::resolve_parts(
                &Default::default(),
                &Default::default(),
                mod_api::__private::KnownIds {
                    block: &|_| true,
                    item: &|_| false,
                },
            )
            .unwrap();
            let inputs = Arc::new(LoadInputs::new(Default::default(), std::env::temp_dir()));
            let context = LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, [])
                .loop_context(LoopSide::Server, [] as [&str; 0]);
            let ask = |state: u32| {
                let mut shape = None;
                collision_shape(
                    &context,
                    BlockPos::new(1, 64, 1),
                    BlockStateId(state),
                    &mod_api::CollisionQuery::default(),
                    &mut shape,
                );
                shape
            };
            DISABLED.with_borrow_mut(HashSet::clear);
            // Asked for stone: the part's boxes, every time, and the part is not disabled.
            assert_eq!(ask(0), Some(vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]));
            assert_eq!(ask(0), Some(vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]));
            assert_eq!(calls[0].load(Ordering::SeqCst), 2);
            // A panic keeps the normal shape (none) and switches the block off: the part is not
            // called a second time.
            assert_eq!(ask(1), None);
            assert_eq!(ask(1), None);
            assert_eq!(calls[1].load(Ordering::SeqCst), 1);
            // A refused answer reaches the adapter as boxes: the one check is made where the shape
            // is built, which reports the refusal here and switches the block off.
            assert!(ask(2).is_some());
            refused(BlockStateId(2), "TooManyBoxes");
            assert_eq!(ask(2), None);
            assert_eq!(calls[2].load(Ordering::SeqCst), 1);
            assert_eq!(DISABLED.with_borrow(HashSet::len), 2);
            // A state with no part is not asked and keeps its shape.
            assert_eq!(ask(3), None);
        });
    }
}
