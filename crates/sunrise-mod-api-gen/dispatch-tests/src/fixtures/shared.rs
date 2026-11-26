//! Shared exception: dispatch infrastructure and cross-area fixture types/tests; no new hooks.
use mod_api::mod_api::{BlockEntityDataView, BlockEntityTypeKey, BlockRemovalEffects};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use sunrise_core::game_mode::GameMode;

#[derive(Clone, Debug, Default, PartialEq)]
struct ItemStack {
    item: Arc<str>,
    count: u32,
}
// Dispatch tests isolate the runtime key table: the fixture key retains its name.
// Production key resolution and slot stability are tested in mod-runtime.
#[derive(Clone, Copy)]
struct ItemKey(&'static str);
impl ItemStack {
    fn count(&self) -> u32 {
        self.count
    }
    fn item(&self) -> ItemKey {
        ItemKey(Box::leak(self.item.to_string().into_boxed_str()))
    }
}
#[derive(Debug)]
enum StackWriteError {
    /// The engine's `CountExceeded { maximum }`, for a fixture validator to refuse an
    /// overstacked edit with (the fixture never reads the maximum).
    CountExceeded,
}
mod content {
    pub(super) mod registry_keys {
        pub(in crate::tests) fn block_key(name: &str) -> Option<super::super::BlockKey> {
            match name {
                "test:block" => Some(super::super::BlockKey::Test),
                "test:other" => Some(super::super::BlockKey::Other),
                _ => None,
            }
        }
        pub(in crate::tests) fn block_entity_type_name(
            _: super::super::BlockEntityTypeKey,
        ) -> Option<&'static str> {
            Some("test:counter")
        }
        pub(in crate::tests) fn item_id(key: super::super::ItemKey) -> Option<&'static str> {
            Some(key.0)
        }
        pub(in crate::tests) fn effect_id(key: super::super::EffectKey) -> Option<&'static str> {
            match key.0 {
                0 => Some("test:effect"),
                _ => None,
            }
        }
    }
}

struct ItemPlace;
struct BlockEntityDraw(Vec<u32>);
use serde_json::{Map, Value};
use stubs::*;
#[derive(Clone, PartialEq)]
struct Text;
#[derive(Debug)]
enum TextFormError {}
#[derive(Clone)]
struct ScreenshotImage;
enum Flow {
    Continue,
    Cancel,
}
trait Host {}
trait ServerHost: Host {
    fn transaction_refusal(
        &mut self,
    ) -> Option<item::transaction::TransactionError<core::convert::Infallible>> {
        None
    }
    fn transaction_matches(&self, tx: Option<&()>) -> bool {
        tx.is_none()
    }
}
trait ClientHost: Host {}

mod client {
    pub(super) mod tooltip_components {
        pub(in crate::tests) fn retain_gather_tooltip_payloads(
            _: &'static str,
            _: &[super::super::TooltipElement],
        ) {
        }
        pub(in crate::tests) fn invoke_tooltip_image<P: ?Sized, R>(
            part: &&P,
            call: impl FnOnce(&P) -> Option<R>,
        ) -> Option<R> {
            let disabled = super::super::DISABLED_BLOCK_PART.get();
            if disabled == Some(std::ptr::from_ref(*part).cast())
                || super::super::runtime::poisoned(super::super::HookId::TooltipImage, "part")
            {
                return None;
            }
            super::super::runtime::invoke(
                super::super::HookId::TooltipImage,
                "tooltip_image",
                "part",
                || call(*part),
            )
            .flatten()
        }
    }
}

mod item {
    pub(super) mod transaction {
        use super::super::ServerHost;

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(in crate::tests) enum TransactionError<E> {
            Aborted(E),
            Unavailable,
            InvalidContext,
            CommitRefused,
        }

        // The game exercises the actual journal; these fixtures have no world state.
        pub(in crate::tests) trait HostTransaction {
            fn transaction<T, E>(
                &mut self,
                call: impl FnOnce(&mut dyn ServerHost, &()) -> Result<T, E>,
            ) -> Result<T, TransactionError<E>>;
        }
        impl HostTransaction for dyn ServerHost + '_ {
            fn transaction<T, E>(
                &mut self,
                call: impl FnOnce(&mut dyn ServerHost, &()) -> Result<T, E>,
            ) -> Result<T, TransactionError<E>> {
                match self.transaction_refusal() {
                    None => call(self, &()).map_err(TransactionError::Aborted),
                    Some(TransactionError::Unavailable) => Err(TransactionError::Unavailable),
                    Some(TransactionError::InvalidContext) => Err(TransactionError::InvalidContext),
                    Some(TransactionError::CommitRefused) => {
                        call(self, &()).map_err(TransactionError::Aborted)?;
                        Err(TransactionError::CommitRefused)
                    }
                    Some(TransactionError::Aborted(never)) => match never {},
                }
            }
        }
    }
}

#[test]
fn transaction_fixture_distinguishes_admission_and_settlement_refusal() {
    use core::convert::Infallible;
    use item::transaction::{HostTransaction, TransactionError};

    struct RefusingHost(Option<TransactionError<Infallible>>);
    impl Host for RefusingHost {}
    impl ServerHost for RefusingHost {
        fn transaction_refusal(&mut self) -> Option<TransactionError<Infallible>> {
            self.0.take()
        }
    }

    for (refusal, ran) in [
        (TransactionError::Unavailable, false),
        (TransactionError::InvalidContext, false),
        (TransactionError::CommitRefused, true),
    ] {
        let mut host = RefusingHost(Some(refusal));
        let mut called = false;
        let result = (&mut host as &mut dyn ServerHost).transaction(|_, _| {
            called = true;
            Ok::<_, Infallible>(())
        });
        assert_eq!(result, Err(refusal));
        assert_eq!(called, ran);
    }
}

/// Owner, hook and field for one generated dispatch refusal.
type OutputRefusal = (&'static str, &'static str, &'static str);
thread_local! {
    static OUTPUT_REFUSALS: std::cell::RefCell<Option<Vec<OutputRefusal>>> = const { std::cell::RefCell::new(None) };
}

fn capture_output_refusals(call: impl FnOnce()) -> Vec<OutputRefusal> {
    struct Restore(Option<Vec<OutputRefusal>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OUTPUT_REFUSALS.replace(self.0.take());
        }
    }
    let _restore = Restore(OUTPUT_REFUSALS.replace(Some(Vec::new())));
    call();
    OUTPUT_REFUSALS.take().unwrap()
}
trait WorldDrawContext {}
trait DrawContext {
    fn mark(&mut self, value: u32);
}
impl DrawContext for HudBatch {
    fn mark(&mut self, value: u32) {
        self.0.push(value);
    }
}
/// Records which part drew, in order.
trait BlockEntityDrawContext {
    fn mark(&mut self, part: u32);
}
impl BlockEntityDrawContext for BlockEntityDraw {
    fn mark(&mut self, part: u32) {
        self.0.push(part);
    }
}
enum HostSide<'a> {
    Server(&'a mut dyn ServerHost),
    Client(&'a mut dyn ClientHost),
}
impl HostSide<'_> {
    fn reborrow(&mut self) -> HostSide<'_> {
        match self {
            Self::Server(host) => HostSide::Server(&mut **host),
            Self::Client(host) => HostSide::Client(&mut **host),
        }
    }
}
enum LevelSide<'a> {
    Server(&'a dyn LevelHost),
    Client(&'a dyn LevelHost),
}
impl LevelSide<'_> {
    fn reborrow(&mut self) -> LevelSide<'_> {
        match self {
            Self::Server(host) => LevelSide::Server(*host),
            Self::Client(host) => LevelSide::Client(*host),
        }
    }
}
enum LevelRandomSide<'a> {
    Server(&'a mut dyn LevelRandomHost),
    Client(&'a dyn LevelHost),
}
impl LevelRandomSide<'_> {
    fn reborrow(&mut self) -> LevelRandomSide<'_> {
        match self {
            Self::Server(host) => LevelRandomSide::Server(&mut **host),
            Self::Client(host) => LevelRandomSide::Client(*host),
        }
    }
}
struct TestHost;
struct TestLevel;
impl LevelHost for TestLevel {}
impl LevelRandomHost for TestLevel {}
impl HitHost for TestLevel {}
impl Host for TestHost {}
impl ServerHost for TestHost {}
impl ClientHost for TestHost {}

#[test]
fn level_random_dispatch_reborrows_the_active_side() {
    let mut host = TestLevel;
    assert!(matches!(
        LevelRandomSide::Server(&mut host).reborrow(),
        LevelRandomSide::Server(_)
    ));
    assert!(matches!(
        LevelRandomSide::Client(&host).reborrow(),
        LevelRandomSide::Client(_)
    ));
}

#[allow(dead_code)]
mod generated {
    pub(super) mod mod_api {
        pub struct Block {
            pub id: String,
        }
        pub struct Item {
            pub id: String,
        }
        pub(super) use super::super::{ItemKey, ItemPlace, ItemStack, ItemStackView};
        // Generated registration dispatch builds the stub `Answered` through this path.
        pub(super) use super::super::Answered;
        pub(super) use super::super::{
            ClientCommandRegistration, CommandRegistration, Dispensed, InteractionResult,
            MapDecoration, MapInfo,
        };
        pub(super) use super::super::{LevelRef, PlayerRef};
        #[derive(Clone)]
        pub struct FluidModel;
        #[derive(Clone, Copy)]
        pub struct EntityRef;
        include!(concat!(env!("OUT_DIR"), "/dispatch_fixtures_api.rs"));
    }
    include!(concat!(env!("OUT_DIR"), "/mod_api_generated.rs"));
    pub(super) use self::mod_api::InlineImageReplacement;
    // `living_heal` and the motion value hooks name the lent view unqualified.
    pub(super) use self::mod_api::LivingView;
    pub(super) use self::mod_api::{BlockFace, BlockKey, BlockState, CrosshairTarget, NameTagLine};
    use self::mod_api::{GuiCanvas, StandaloneModelRegistration};
    pub use sunrise_mod_api_macros::__subscriptions;
}
use generated::*;
struct Installed {
    mods: Arc<[ModEntry]>,
    subscribers: sunrise_mod_runtime::subscribers::Subscribers,
}
thread_local! {
    /// The hooks each dispatch entered, a mod at a time.
    static ENTERED: std::cell::RefCell<Vec<(usize, &'static str)>> = const { std::cell::RefCell::new(Vec::new()) };
    static MODS: std::cell::Cell<Option<&'static Installed>> = const { std::cell::Cell::new(None) };
}
fn install(mods: Arc<[ModEntry]>) {
    let subscriptions = mods.iter().enumerate().flat_map(|(index, entry)| {
        let listeners = entry.hooks.listeners();
        entry.hooks.subscribed().iter().flatten().map(move |hook| {
            let listener = listeners.iter().find(|listener| {
                listener
                    .hook
                    .is_some_and(|id| id as usize == *hook as usize)
            });
            sunrise_mod_runtime::subscribers::Subscription {
                index,
                hook: *hook as usize,
                priority: listener.map_or(0, |listener| listener.priority),
                receives_cancelled: listener.is_some_and(|listener| listener.receive_cancelled),
            }
        })
    });
    let subscribers =
        sunrise_mod_runtime::subscribers::Subscribers::new(HOOK_COUNT, mods.len(), subscriptions);
    // A fresh installation starts poison-free, as the real runtime's does per session.
    runtime::POISONED.with(|poisoned| poisoned.borrow_mut().clear());
    // Test dispatch needs the same static lifetime as the real installation.
    MODS.set(Some(Box::leak(Box::new(Installed { mods, subscribers }))));
}
fn listened(hook: HookId) -> bool {
    MODS.get()
        .is_some_and(|installed| installed.subscribers.contains_any(hook as usize))
}
fn receives_cancelled(hook: HookId) -> bool {
    MODS.get()
        .is_some_and(|installed| installed.subscribers.receives_cancelled(hook as usize))
}
fn entered_with_id(
    hook_id: HookId,
    hook: &'static str,
) -> impl Iterator<Item = (&'static str, &'static dyn Hooks)> {
    runtime::entered_with_id_matching(hook_id, hook, |_| true)
}
fn listening(hook_id: HookId) -> impl Iterator<Item = (bool, &'static ModEntry)> {
    MODS.get().into_iter().flat_map(move |installed| {
        installed
            .subscribers
            .subscribers(hook_id as usize)
            .filter(move |subscriber| {
                !runtime::poisoned(hook_id, installed.mods[subscriber.index].id)
            })
            .map(move |subscriber| {
                (
                    subscriber.receives_cancelled,
                    &installed.mods[subscriber.index],
                )
            })
    })
}
fn enter_listener(entry: &'static ModEntry, hook: &'static str) -> ((), &'static dyn Hooks) {
    let index = MODS
        .get()
        .unwrap()
        .mods
        .iter()
        .position(|e| std::ptr::eq(e, entry))
        .unwrap();
    ENTERED.with(|entered| entered.borrow_mut().push((index, hook)));
    ((), &*entry.hooks)
}
#[allow(dead_code)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Side {
    Both,
    Client,
    Server,
}
struct ModEntry {
    id: &'static str,
    hooks: Box<dyn Hooks>,
    side: Side,
}
impl ModEntry {
    fn new(id: &'static str, hooks: Box<dyn Hooks>) -> Self {
        Self {
            id,
            hooks,
            side: Side::Both,
        }
    }
}
trait Registrar {}
/// The goal registrar of the generated `EntityPart`, as the fixture sees it.
struct GoalRegistrar<'a> {
    _lifetime: std::marker::PhantomData<&'a ()>,
}
impl GoalRegistrar<'_> {
    fn route_kind(&self) -> &str {
        "test:mob"
    }
    fn set_owner(&mut self, _: &'static str, _: &'static str) {}
    fn checkpoint(&self) -> usize {
        0
    }
    fn rollback(&mut self, _: usize) {}
}
/// The behavior registrar of the generated `EntityPart` (#824), as the fixture sees it.
struct BehaviorRegistrar<'a> {
    _lifetime: std::marker::PhantomData<&'a ()>,
}
impl BehaviorRegistrar<'_> {
    fn route_kind(&self) -> &str {
        "test:mob"
    }
    fn set_owner(&mut self, _: &'static str, _: &'static str) {}
    fn checkpoint(&self) -> usize {
        0
    }
    fn rollback(&mut self, _: usize) {}
}
/// Scope fixtures install their own Parts. Block attachments retain their API keys;
/// other scopes keep their existing single-target fixture tables.
#[derive(Default)]
struct Parts {
    blocks: Vec<(BlockKey, Vec<&'static dyn BlockPart>)>,
    owner_slot: Option<(BlockKey, usize)>,
    block_entities: Vec<&'static dyn BlockEntityPart>,
    items: Vec<&'static dyn ItemPart>,
    effects: Vec<&'static dyn MobEffectPart>,
    menus: Vec<&'static dyn MenuPart>,
    fluids: Vec<&'static dyn FluidPart>,
    entities: Vec<&'static dyn EntityPart>,
}
static NO_PARTS: Parts = Parts {
    blocks: Vec::new(),
    owner_slot: None,
    block_entities: Vec::new(),
    items: Vec::new(),
    effects: Vec::new(),
    menus: Vec::new(),
    fluids: Vec::new(),
    entities: Vec::new(),
};
thread_local! {
    static PARTS: std::cell::Cell<&'static Parts> = const { std::cell::Cell::new(&NO_PARTS) };
}
fn install_parts(parts: Parts) {
    PARTS.set(Box::leak(Box::new(parts)));
}
/// `part` for the parts list of a test: parts live as long as the installation.
fn leak<T>(part: T) -> &'static T {
    Box::leak(Box::new(part))
}
fn has_part_hook(_: HookId) -> bool {
    // Manual fixture Parts override methods without emitting subscription bits.
    // Their scope tables still refuse an empty installation and unrelated block keys.
    let parts = PARTS.get();
    !parts.blocks.is_empty()
        || !parts.block_entities.is_empty()
        || !parts.items.is_empty()
        || !parts.effects.is_empty()
        || !parts.menus.is_empty()
        || !parts.entities.is_empty()
}
fn parts(_: HookId, _: &'static str) -> &'static Parts {
    PARTS.get()
}
impl Parts {
    fn for_hook(&self, _: HookId) -> Option<&Self> {
        Some(self)
    }
    fn block_value(&self, state: BlockState) -> &[&'static dyn BlockPart] {
        // Both fixture block definitions have two local state ordinals, 0 and 1.
        if state.ordinal() > 1 {
            return &[];
        }
        self.block_key(Some(state.block()))
    }
    fn block_entry(&self, state: BlockState) -> Option<BlockHookParts<'_>> {
        let parts = self.block_value(state);
        (!parts.is_empty()).then_some(BlockHookParts {
            parts,
            owner_slot: self
                .owner_slot
                .filter(|(key, _)| *key == state.block())
                .map(|(_, slot)| slot),
        })
    }
    fn block_key(&self, key: Option<BlockKey>) -> &[&'static dyn BlockPart] {
        key.and_then(|key| self.blocks.iter().find(|(block, _)| *block == key))
            .map_or(&[], |(_, parts)| parts.as_slice())
    }
    fn block_entity<'k>(
        &self,
        _: impl FnOnce() -> Option<&'k str>,
    ) -> &[&'static dyn BlockEntityPart] {
        &self.block_entities
    }
    fn block_entity_kind<'k>(
        &self,
        _: impl FnOnce() -> Option<&'k str>,
    ) -> &[&'static dyn BlockEntityPart] {
        &self.block_entities
    }
    fn item<'k>(&self, _: impl FnOnce() -> Option<&'k str>) -> &[&'static dyn ItemPart] {
        &self.items
    }
    fn effect<'k>(&self, id: impl FnOnce() -> Option<&'k str>) -> &[&'static dyn MobEffectPart] {
        match id() {
            Some("test:effect") => &self.effects,
            _ => &[],
        }
    }
    fn menu<'k>(&self, _: impl FnOnce() -> Option<&'k str>) -> &[&'static dyn MenuPart] {
        &self.menus
    }
    fn menu_for_opener(&self, _: &str, _: Option<&str>) -> &[&'static dyn MenuPart] {
        &self.menus
    }
    fn fluid<'k>(&self, _: impl FnOnce() -> Option<&'k str>) -> &[&'static dyn FluidPart] {
        &self.fluids
    }
    fn entity<'k>(&self, _: impl FnOnce() -> Option<&'k str>) -> &[&'static dyn EntityPart] {
        &self.entities
    }
}
struct BlockHookParts<'a> {
    parts: &'a [&'static dyn BlockPart],
    owner_slot: Option<usize>,
}
impl BlockHookParts<'_> {
    fn parts(&self) -> &[&'static dyn BlockPart] {
        self.parts
    }
    fn disabled_owner_slot(&self) -> Option<usize> {
        self.owner_slot.filter(|&slot| {
            DISABLED_BLOCK_PART.get() == Some(std::ptr::from_ref(self.parts[slot]).cast())
        })
    }
}
thread_local! {
    // These fixtures use bare trait references, with at most one disabled attachment.
    static DISABLED_BLOCK_PART: std::cell::Cell<Option<*const ()>> = const { std::cell::Cell::new(None) };
}
fn any_mod_disabled() -> bool {
    DISABLED_BLOCK_PART.get().is_some()
}
fn entered_parts<'a, P: ?Sized>(
    hook_id: HookId,
    parts: &'a [&'a P],
    _: &'static str,
) -> impl Iterator<Item = (&'static str, &'a P)> {
    let disabled = DISABLED_BLOCK_PART.get();
    parts
        .iter()
        .copied()
        .filter(move |part| {
            disabled != Some(std::ptr::from_ref(*part).cast())
                && !runtime::poisoned(hook_id, "part")
        })
        .map(|part| ("part", part))
}
fn entered_parts_with_owner<'a, P: ?Sized>(
    hook: generated::HookId,
    parts: &'a [&'a P],
    name: &'static str,
) -> impl Iterator<Item = (&'static str, &'static str, &'a P)> {
    entered_parts(hook, parts, name).map(|(id, part)| (id, "Part", part))
}
fn entered_parts_with_slot<'a, P: ?Sized>(
    _: generated::HookId,
    parts: &'a [&'a P],
    _: &'static str,
) -> impl Iterator<Item = (&'static str, (usize, usize), &'a P)> {
    let disabled = DISABLED_BLOCK_PART.get();
    parts
        .iter()
        .copied()
        .enumerate()
        .filter(move |(_, part)| disabled != Some(std::ptr::from_ref(*part).cast()))
        .map(|(order, part)| ("part", (0, order), part))
}
fn part_slot<'a, P: ?Sized>(parts: &'a [&'a P], slot: (usize, usize)) -> Option<&'a [&'a P]> {
    match slot {
        (0, order) => parts.get(order).map(std::slice::from_ref),
        _ => None,
    }
}

#[test]
fn a_disabled_owner_answers_only_at_its_dispatch_position() {
    struct Probe(Option<InteractionResult>, std::sync::atomic::AtomicUsize);
    impl BlockPart for Probe {
        fn block_subscribed(&self) -> &'static [Option<HookId>] {
            &[]
        }
        fn use_without_item(
            &self,
            _: HostSide<'_>,
            _: &mut UseWithoutItemEvent<'_>,
        ) -> Option<InteractionResult> {
            self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.0
        }
    }
    let before = leak(Probe(
        Some(InteractionResult::SuccessServer),
        Default::default(),
    ));
    let declines = leak(Probe(None, Default::default()));
    let owner: &dyn BlockPart = leak(Probe(None, Default::default()));
    let after = leak(Probe(
        Some(InteractionResult::SuccessServer),
        Default::default(),
    ));
    DISABLED_BLOCK_PART.set(Some(std::ptr::from_ref(owner).cast()));
    let interact = |first: &'static dyn BlockPart| {
        install_parts(Parts {
            blocks: vec![(BlockKey::Test, vec![first, owner, after])],
            owner_slot: Some((BlockKey::Test, 1)),
            ..Parts::default()
        });
        let mut answer = None;
        dispatch_use_without_item(
            HostSide::Server(&mut TestHost),
            BlockState::new(BlockKey::Test, 0),
            &BlockClick(&()),
            &mut answer,
        );
        answer
    };
    assert_eq!(interact(before), Some(InteractionResult::SuccessServer));
    assert_eq!(interact(declines), Some(InteractionResult::Pass));
    use std::sync::atomic::Ordering::Relaxed;
    assert_eq!(before.1.load(Relaxed), 1);
    assert_eq!(declines.1.load(Relaxed), 1);
    assert_eq!(after.1.load(Relaxed), 0);
    DISABLED_BLOCK_PART.set(None);
}

#[test]
fn inert_growth_answers_keep_subscribed_crop_and_soil() {
    struct Worldgen;
    impl WorldgenHost for Worldgen {}
    struct Draws;
    impl RandomSource for Draws {}
    let owner: &dyn BlockPart = leak(Mod {
        id: 0,
        calls: Arc::default(),
    });
    let non_owner: &dyn BlockPart = leak(Mod {
        id: 1,
        calls: Arc::default(),
    });
    DISABLED_BLOCK_PART.set(Some(std::ptr::from_ref(owner).cast()));
    install_parts(Parts {
        owner_slot: Some((BlockKey::Test, 0)),
        blocks: vec![
            (BlockKey::Test, vec![owner]),
            (BlockKey::Other, vec![non_owner]),
        ],
        ..Parts::default()
    });
    WORLDGEN_PARTS.set(PARTS.get());
    for (key, expected_valid, expected_soil) in [
        (BlockKey::Test, Some(false), Some(None)),
        (BlockKey::Other, None, None),
    ] {
        let state = BlockState::new(key, 0);
        let mut valid = None;
        dispatch_is_valid_bonemeal_target(
            LevelSide::Server(&TestLevel),
            LevelDimension,
            BlockPos,
            state,
            &mut valid,
        );
        assert_eq!(valid, expected_valid);
        let mut soil = None;
        dispatch_on_tree_grow(&Worldgen, state, [1, 64, 2], &mut Draws, &mut soil);
        assert_eq!(soil, expected_soil);
    }
    DISABLED_BLOCK_PART.set(None);
}

#[test]
fn inert_support_signals_that_the_simulation_decides() {
    let owner: &dyn BlockPart = leak(Mod {
        id: 0,
        calls: Arc::default(),
    });
    DISABLED_BLOCK_PART.set(Some(std::ptr::from_ref(owner).cast()));
    install_parts(Parts {
        blocks: vec![(BlockKey::Test, vec![owner])],
        owner_slot: Some((BlockKey::Test, 0)),
        ..Parts::default()
    });
    let other = BlockState::new(BlockKey::Other, 0);
    for (key, disabled) in [(BlockKey::Test, true), (BlockKey::Other, false)] {
        let state = BlockState::new(key, 0);
        let (mut survives, mut inert) = (None, false);
        dispatch_can_survive(
            LevelSide::Server(&TestLevel),
            LevelDimension,
            BlockPos,
            state,
            &mut survives,
            &mut inert,
        );
        assert_eq!((survives, inert), (None, disabled));

        let (mut may, mut inert) = (None, false);
        dispatch_may_place_on(
            LevelSide::Server(&TestLevel),
            LevelDimension,
            state,
            other,
            BlockPos,
            &mut may,
            &mut inert,
        );
        assert_eq!((may, inert), (None, disabled));

        let (mut sustains, mut inert) = (None, false);
        dispatch_can_sustain_plant(
            LevelSide::Server(&TestLevel),
            LevelDimension,
            other,
            state,
            BlockPos,
            &mut sustains,
            &mut inert,
        );
        assert_eq!((sustains, inert), (None, disabled));
    }
    DISABLED_BLOCK_PART.set(None);
}

#[test]
fn cfg_disabled_hooks_are_not_subscribed_and_cfg_attr_stays() {
    struct Gated;
    #[mod_api::__subscriptions]
    impl Hooks for Gated {
        #[cfg(any())]
        fn loot_table_load(&self, _: &dyn DataLoadContext, _: &mut LootTableLoadEvent<'_>) {
            unreachable!()
        }
        #[cfg(not(any()))]
        fn player_logged_in(&self, _: &mut dyn ServerHost, _: &mut PlayerLoggedInEvent<'_>) {}
        #[cfg_attr(test, inline)]
        fn player_logged_out(&self, _: &mut dyn ServerHost, _: &mut PlayerLoggedOutEvent<'_>) {}
    }
    install(vec![ModEntry::new("gated", Box::new(Gated))].into());
    assert!(
        !listened(HookId::LootTableLoad),
        "a cfg-disabled hook was subscribed"
    );
    assert!(
        listened(HookId::PlayerLoggedIn),
        "a cfg-enabled hook was not subscribed"
    );
    assert!(
        listened(HookId::PlayerLoggedOut),
        "cfg_attr never unsubscribes"
    );
}

struct Mod {
    id: u32,
    calls: Arc<Mutex<Vec<u32>>>,
}
impl BlockPart for Mod {
    fn block_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
}
impl BlockEntityPart for Mod {
    fn block_entity_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
    fn block_entity_submit(&self, _: &dyn ClientHost, event: &mut BlockEntitySubmitEvent<'_>) {
        assert_eq!(event.pos, [1, 2, 3]);
        assert_eq!(event.block, "example:machine");
        assert_eq!(event.entity.id(), "example:subject");
        event.draw.mark(self.id);
    }
}
impl MenuPart for Mod {
    fn menu_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
    fn menu_button(&self, event: &mut MenuButtonEvent<'_>) {
        assert_eq!(event.context.player, 123);
        assert_eq!(event.context.player_entity_id, 42);
        assert_eq!(event.context.id, 9);
        let pos = event.context.pos;
        if event
            .subject
            .with_view(|entity| {
                assert_eq!(pos, Some([1, 2, 3]));
                assert_eq!(entity.id(), "machine");
                entity.data.push(9);
            })
            .is_none()
        {
            assert_eq!(pos, None);
        }
        *event.carried = Some(ItemStack {
            item: "bucket".into(),
            count: 1,
        });
    }
    fn menu_slot_rules(&self, _: &dyn Host, event: &mut MenuSlotRulesEvent<'_>) {
        // The context arrives as the game passed it: who, where and the other slots.
        let context = event.context;
        assert_eq!((context.player, context.pos), (Some(7), Some([1, 2, 3])));
        assert_eq!(event.container, MenuContainer::Grid);
        assert!(context.slot(MenuContainer::Grid, 0).is_none());
        let stack = context.slot(MenuContainer::Grid, 1).unwrap();
        assert_eq!((stack.item.as_ref(), stack.count), ("planks", 4));
        event.may_place = false;
        event.max_stack = self.id;
    }
}
impl EntityPart for Mod {
    fn entity_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
    fn general_entity_tick(&self, event: &mut GeneralEntityTickEvent<'_>) {
        assert_eq!(event.kind, "mymod:robot");
        self.calls.lock().unwrap().push(self.id);
    }
}
impl ItemPart for Mod {
    fn item_subscribed(&self) -> &'static [Option<HookId>] {
        &[]
    }
    fn tooltip(&self, _: &dyn ClientHost, event: &mut TooltipEvent<'_, '_>) {
        event.lines.push((Text, self.id)).unwrap();
    }
    fn use_on(&self, _: HostSide<'_>, _: &mut UseOnEvent<'_>) -> Option<InteractionResult> {
        self.calls.lock().unwrap().push(self.id);
        if self.id == 1 {
            Some(InteractionResult::SuccessServer)
        } else {
            None
        }
    }
}
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn living_entity_use_item_finish(
        &self,
        _: HostSide,
        event: &mut LivingEntityUseItemFinishEvent<'_>,
    ) {
        *event.result = None;
    }
    fn mouse_button_pre(&self, _: &mut dyn ClientHost, _: &mut MouseButtonPreEvent) -> Flow {
        self.calls.lock().unwrap().push(self.id);
        if self.id == 1 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn living_entity_use_item_tick(
        &self,
        _: HostSide,
        event: &mut LivingEntityUseItemTickEvent<'_>,
    ) -> Flow {
        event.duration -= 1;
        if self.id == 1 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn loot_table_load(&self, _: &dyn DataLoadContext, event: &mut LootTableLoadEvent<'_>) {
        event.table.as_array_mut().unwrap().push(self.id.into());
    }
    fn modify_default_components(
        &self,
        context: &dyn DataLoadContext,
        event: &mut ModifyDefaultComponentsEvent<'_>,
    ) {
        assert_eq!(context.origin(), "defaults");
        let value = event
            .components
            .entry("order")
            .or_insert_with(|| serde_json::json!([]));
        value.as_array_mut().unwrap().push(self.id.into());
    }
    fn level_tick_pre(&self, _: HostSide, _: &mut LevelTickPreEvent) {
        self.calls.lock().unwrap().push(self.id);
    }
    fn prepare_item_craft(
        &self,
        _: &mut dyn ServerHost,
        _: &mut PrepareItemCraftEvent<'_>,
    ) -> Option<ItemStack> {
        self.calls.lock().unwrap().push(self.id);
        Some(ItemStack {
            item: "test:crafted".into(),
            count: self.id,
        })
    }
    fn living_incoming_damage(
        &self,
        _: &mut dyn HitHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> Flow {
        *event.container.0 = *event.container.0 * 2.0 + self.id as f32;
        if self.id == 2 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}
fn part(id: u32, calls: &Arc<Mutex<Vec<u32>>>) -> &'static Mod {
    leak(Mod {
        id,
        calls: calls.clone(),
    })
}

// The vocabulary's own list type, so a list hook's fixture drives the real journal; the
// fixtures use only part of it.
#[allow(dead_code)]
mod hook_list {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../sunrise-mod-vocab/src/hook_list.rs"
    ));
}
mod stubs {
    pub(super) use super::generated::mod_api::LivingRef;
    pub(super) use super::hook_list::{HookList, HookListJournal, HookListStorage};
    pub(super) use crate::tests::mod_api::mod_api::DamageContainer;
}
mod crate_root {
    // Dispatch fixtures isolate ordering; the vocabulary's own tests cover what the client
    // validators refuse. One module for every client hook's validator.
    mod client {
        pub(crate) mod hud_layout {
            pub(crate) fn validate_hotbar_attack_indicator_progress(
                _: &f32,
                _: &f32,
            ) -> Result<(), &'static str> {
                Ok(())
            }
        }
    }
}
