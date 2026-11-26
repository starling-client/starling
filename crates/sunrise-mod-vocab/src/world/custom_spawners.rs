//! A level's custom spawners (`CustomSpawner`): the list `ModifyCustomSpawnersEvent` edits.
//!
//! Vanilla `ServerLevel` owns a list of `CustomSpawner` instances (`MinecraftServer#createLevels`
//! populates the Overworld with its five built-ins and other dimensions with an empty list) and
//! ticks each once per eligible server tick (`ServerLevel#tickCustomSpawners`). Mods edit that
//! list once per level in [`Hooks::modify_custom_spawners`](crate::Hooks::modify_custom_spawners),
//! removing built-ins or appending owned [`CustomSpawner`] objects directly.
use crate::callback_owner::CallbackOwner;
use crate::{LevelRef, ServerHost};
use std::{any::TypeId, fmt};

/// A built-in spawner vanilla runs for the Overworld (`MinecraftServer#createLevels`).
// no wire: server-side custom spawner selector, not a serialized payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SpawnerKind {
    /// `PhantomSpawner`: phantoms for players who have not slept.
    Phantom,
    /// `PatrolSpawner`: pillager patrols.
    Patrol,
    /// `CatSpawner`: cats in villages and swamp huts.
    Cat,
    /// `VillageSiege`: zombie sieges on a village at night.
    VillageSiege,
    /// `WanderingTraderSpawner`: the wandering trader and its llamas.
    WanderingTrader,
}

/// An owned custom spawner ticked once per eligible server level tick
/// (`net.minecraft.world.level.CustomSpawner`).
///
/// Register an instance once per level in
/// [`Hooks::modify_custom_spawners`](crate::Hooks::modify_custom_spawners) with
/// [`CustomSpawners::add`]. The level owns the instance for its lifetime and drops it when the
/// level unloads or the world closes. Each tick runs as the registering mod inside a server
/// transaction. A panic rolls back that tick's writes, disables this spawner type for the
/// registering mod for the rest of the session, logs loudly and notifies the client error toast
/// queue. Ordinary Host errors do not disable a spawner.
/// Dropping a spawner also runs under the registering mod's panic containment.
// no wire: owned server-side spawner object with local state and tick callback.
pub trait CustomSpawner: Send + 'static {
    /// Vanilla `CustomSpawner#tick(ServerLevel, boolean)`: run one custom-spawner tick on
    /// `level` outside debug worlds while the `spawn_mobs` game rule is enabled
    /// (`ServerChunkCache#tickChunks`). `spawn_enemies` is the chunk source's stored
    /// setting. The server samples `spawn_mobs && spawn_monsters` at startup, on difficulty
    /// updates, and when `spawn_monsters` is set (`MinecraftServer#updateMobSpawningFlags`).
    /// Changing only `spawn_mobs` changes tick eligibility, not the stored setting.
    /// Peaceful is not a separate filter on that setting.
    fn tick(&mut self, host: &mut dyn ServerHost, level: LevelRef, spawn_enemies: bool);
}

impl<F> CustomSpawner for F
where
    F: FnMut(&mut dyn ServerHost, LevelRef, bool) + Send + 'static,
{
    fn tick(&mut self, host: &mut dyn ServerHost, level: LevelRef, spawn_enemies: bool) {
        self(host, level, spawn_enemies);
    }
}

/// One level-owned mod custom spawner with its registering mod's session owner.
// no wire: owns a server-side spawner object and its process-local mod identity.
// hidden: the game ticks registered spawners; mods register through CustomSpawners::add.
#[doc(hidden)]
#[non_exhaustive]
pub struct RegisteredCustomSpawner {
    spawner: Option<Box<dyn CustomSpawner>>,
    owner: CallbackOwner,
}

impl RegisteredCustomSpawner {
    /// Whether this spawner's registering mod and concrete type are still enabled this session.
    pub fn is_enabled(&self) -> bool {
        self.owner.is_enabled()
    }

    /// The registering mod's identifier.
    pub fn owner_id(&self) -> &'static str {
        self.owner.id()
    }

    /// Tick this spawner once as its registering mod.
    pub fn tick(
        &mut self,
        host: &mut dyn ServerHost,
        level: LevelRef,
        spawn_enemies: bool,
    ) -> Result<(), String> {
        let spawner = self.spawner.as_deref_mut().expect("live custom spawner");
        self.owner
            .call("custom spawner", "CustomSpawner::tick", || {
                spawner.tick(host, level, spawn_enemies)
            })
    }
}

impl Drop for RegisteredCustomSpawner {
    fn drop(&mut self) {
        if let Some(spawner) = self.spawner.take() {
            self.owner
                .dispose("custom spawner", "CustomSpawner::drop", spawner);
        }
    }
}

/// One step taken from a level's custom spawner list during `ServerLevel#tickCustomSpawners`.
// no wire: in-process dispatch step carrying a temporarily taken custom spawner.
// hidden: used only by the game's custom spawner tick loop.
#[doc(hidden)]
#[non_exhaustive]
pub enum CustomSpawnerStep {
    /// Run a vanilla built-in spawner.
    Builtin(SpawnerKind),
    /// Run a mod-owned custom spawner, then return it via [`CustomSpawners::put_back`].
    Mod(RegisteredCustomSpawner),
    /// Skip a disabled, poisoned or currently re-entered mod spawner slot.
    Skip,
}

enum SpawnerSlot {
    Builtin(SpawnerKind),
    Mod(Option<RegisteredCustomSpawner>),
}

/// A level's custom spawners, in the order they tick, owned by the level: the engine's list.
/// The Overworld starts with vanilla's five built-ins, other dimensions start empty (matching
/// `MinecraftServer#createLevels`). A mod edits it only through [`CustomSpawnersEditor`].
// no wire: owns per-level server-side CustomSpawner objects.
// hidden: the game owns each level's list and ticks it; mods edit it through the editor.
#[doc(hidden)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct CustomSpawners {
    entries: Vec<SpawnerSlot>,
}

impl CustomSpawners {
    /// The Overworld's initial list: vanilla's five spawners in the order Sunrise ticks them.
    pub fn vanilla() -> Self {
        use SpawnerKind::*;
        Self {
            entries: [WanderingTrader, Patrol, Cat, VillageSiege, Phantom]
                .into_iter()
                .map(SpawnerSlot::Builtin)
                .collect(),
        }
    }

    /// An empty spawner list, as vanilla creates for non-Overworld dimensions
    /// (`MinecraftServer#createLevels`).
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Whether the built-in spawner `kind` is currently in the list.
    pub fn contains(&self, kind: SpawnerKind) -> bool {
        self.entries
            .iter()
            .any(|slot| matches!(slot, SpawnerSlot::Builtin(k) if *k == kind))
    }

    /// The built-in spawners remaining in this list, in tick order.
    pub fn builtins(&self) -> impl Iterator<Item = SpawnerKind> + '_ {
        self.entries.iter().filter_map(|slot| match slot {
            SpawnerSlot::Builtin(kind) => Some(*kind),
            SpawnerSlot::Mod(_) => None,
        })
    }

    /// Total number of spawners in the list (built-in plus mod-registered).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the list contains no spawners.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Take the entry at `index` for one tick step without holding a borrow on the list.
    pub fn take_at(&mut self, index: usize) -> Option<CustomSpawnerStep> {
        match self.entries.get_mut(index)? {
            SpawnerSlot::Builtin(kind) => Some(CustomSpawnerStep::Builtin(*kind)),
            SpawnerSlot::Mod(slot) => {
                if slot
                    .as_ref()
                    .is_some_and(RegisteredCustomSpawner::is_enabled)
                {
                    Some(CustomSpawnerStep::Mod(slot.take().unwrap()))
                } else {
                    Some(CustomSpawnerStep::Skip)
                }
            }
        }
    }

    /// Return a mod spawner taken by [`Self::take_at`] back to its slot after ticking.
    pub fn put_back(&mut self, index: usize, spawner: RegisteredCustomSpawner) {
        if let Some(SpawnerSlot::Mod(slot @ None)) = self.entries.get_mut(index) {
            *slot = Some(spawner);
        }
    }
}

/// What stands at one place of the list a [`CustomSpawnersEditor`] shows.
// no wire: a borrowed view of a server-side list entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpawnerEntry<'a> {
    /// One of vanilla's built-in spawners.
    Builtin(SpawnerKind),
    /// A spawner a mod added, and the mod that did.
    Mod {
        /// The registering mod's identifier.
        owner: &'a str,
    },
}

/// One place of the editor's staged order: an instance still in the level's list, or one
/// this event added.
#[derive(Clone, Copy)]
enum Staged {
    Existing(usize),
    Added(usize),
}

/// Engine checkpoint for rolling back one listener's edits to a [`CustomSpawnersEditor`].
// no wire: process-local rollback token for one modify_custom_spawners listener.
// hidden: used only by generated modify_custom_spawners dispatch.
// exhaustive: private fields prevent downstream construction.
#[doc(hidden)]
pub struct CustomSpawnersCheckpoint {
    order: Vec<Staged>,
    added: usize,
}

/// The ordered list of a level's custom spawners, as `ModifyCustomSpawnersEvent` edits it
/// (NeoForge hands the listener the level's mutable `CustomSpawner` list). The editor shows
/// vanilla's built-ins and every mod's spawners in tick order, and can insert, remove and move
/// any of them. It does not own the live list: an edit is staged here, a listener that panics
/// has its edits undone, and the game applies what was staged only after every listener ran, so
/// an instance a failing listener removed is still in the level, with its state.
// no wire: stages edits to the level's server-side spawner objects.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct CustomSpawnersEditor {
    /// What stands in the level's list now, by place: only who it is, never the instance.
    existing: Vec<ExistingEntry>,
    order: Vec<Staged>,
    added: Vec<Option<RegisteredCustomSpawner>>,
}

/// Who stands at one place of the level's list.
#[derive(Clone, Copy)]
enum ExistingEntry {
    Builtin(SpawnerKind),
    Mod(&'static str),
}

/// A registered spawner owned by the mod that is running, or None when its type was disabled
/// by an earlier panic this session (the spawner is then dropped under that mod's containment).
fn register<S: CustomSpawner>(spawner: S) -> Option<RegisteredCustomSpawner> {
    let owner = crate::content::crash::current_native_mod()
        .zip(crate::runtime::installed())
        .map_or_else(
            || CallbackOwner::for_mod("unknown"),
            |(mod_id, installed)| {
                CallbackOwner::for_installed_type(
                    installed,
                    mod_id,
                    "custom spawner",
                    TypeId::of::<S>(),
                )
            },
        );
    if !owner.is_enabled() {
        owner.dispose("custom spawner", "CustomSpawner::drop", spawner);
        return None;
    }
    Some(RegisteredCustomSpawner {
        spawner: Some(Box::new(spawner)),
        owner,
    })
}

impl CustomSpawnersEditor {
    /// An editor over what `list` holds, showing its entries in order and changing nothing
    /// until [`Self::commit`].
    // hidden: the game builds the editor for the event
    #[doc(hidden)]
    pub fn new(list: &CustomSpawners) -> Self {
        Self {
            existing: list
                .entries
                .iter()
                .map(|slot| match slot {
                    SpawnerSlot::Builtin(kind) => ExistingEntry::Builtin(*kind),
                    SpawnerSlot::Mod(spawner) => {
                        ExistingEntry::Mod(spawner.as_ref().map_or("unknown", |s| s.owner_id()))
                    }
                })
                .collect(),
            order: (0..list.entries.len()).map(Staged::Existing).collect(),
            added: Vec::new(),
        }
    }

    fn entry_of(&self, place: Staged) -> SpawnerEntry<'_> {
        match place {
            Staged::Existing(index) => match self.existing[index] {
                ExistingEntry::Builtin(kind) => SpawnerEntry::Builtin(kind),
                ExistingEntry::Mod(owner) => SpawnerEntry::Mod { owner },
            },
            Staged::Added(index) => SpawnerEntry::Mod {
                owner: self.added[index]
                    .as_ref()
                    .map_or("unknown", |s| s.owner_id()),
            },
        }
    }

    /// The entry at `index` in tick order.
    pub fn get(&self, index: usize) -> Option<SpawnerEntry<'_>> {
        self.order.get(index).map(|&place| self.entry_of(place))
    }

    /// Every entry, in tick order.
    pub fn entries(&self) -> impl Iterator<Item = SpawnerEntry<'_>> + '_ {
        self.order.iter().map(|&place| self.entry_of(place))
    }

    /// The place of the first built-in `kind`, if it is in the list.
    pub fn position(&self, kind: SpawnerKind) -> Option<usize> {
        self.entries()
            .position(|entry| entry == SpawnerEntry::Builtin(kind))
    }

    /// Whether the built-in spawner `kind` is in the list.
    pub fn contains(&self, kind: SpawnerKind) -> bool {
        self.position(kind).is_some()
    }

    /// The built-in spawners in the list, in tick order.
    pub fn builtins(&self) -> impl Iterator<Item = SpawnerKind> + '_ {
        self.entries().filter_map(|entry| match entry {
            SpawnerEntry::Builtin(kind) => Some(kind),
            SpawnerEntry::Mod { .. } => None,
        })
    }

    /// Total number of entries (built-in plus mod-registered).
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Inserts an owned mod spawner so it stands at `index` (0 puts it before every built-in),
    /// and returns whether it did: false past the end of the list, or when this spawner type was
    /// disabled for the registering mod after an earlier panic this session. The spawner is
    /// dropped under the mod's panic containment when it is not added.
    pub fn insert<S: CustomSpawner>(&mut self, index: usize, spawner: S) -> bool {
        if index > self.order.len() {
            drop(register(spawner));
            return false;
        }
        let Some(registered) = register(spawner) else {
            return false;
        };
        self.added.push(Some(registered));
        self.order
            .insert(index, Staged::Added(self.added.len() - 1));
        true
    }

    /// Appends an owned mod spawner at the end of this level's list (see [`Self::insert`]).
    pub fn add<S: CustomSpawner>(&mut self, spawner: S) {
        self.insert(self.order.len(), spawner);
    }

    /// Takes every built-in spawner `kind` out so it never ticks; returns `false` if it was not
    /// in the list.
    pub fn remove(&mut self, kind: SpawnerKind) -> bool {
        let before = self.order.len();
        let existing = &self.existing;
        self.order.retain(|place| {
            !matches!(place, Staged::Existing(index)
                if matches!(existing[*index], ExistingEntry::Builtin(k) if k == kind))
        });
        self.order.len() != before
    }

    /// Takes the entry at `index` out of the list, a built-in or a mod's instance, and returns
    /// whether there was one. An instance goes (and is dropped) only once every listener ran.
    pub fn remove_at(&mut self, index: usize) -> bool {
        if index >= self.order.len() {
            return false;
        }
        self.order.remove(index);
        true
    }

    /// Moves the entry at `from` so it stands at `to` once it is out of its old place, and
    /// returns whether both places exist.
    pub fn move_entry(&mut self, from: usize, to: usize) -> bool {
        if from >= self.order.len() || to >= self.order.len() {
            return false;
        }
        let place = self.order.remove(from);
        self.order.insert(to, place);
        true
    }

    /// Snapshot the staged edits before one `modify_custom_spawners` listener runs, so a
    /// panic undoes only that listener's edits.
    // hidden: used only by generated modify_custom_spawners dispatch
    #[doc(hidden)]
    pub fn checkpoint(&self) -> CustomSpawnersCheckpoint {
        CustomSpawnersCheckpoint {
            order: self.order.clone(),
            added: self.added.len(),
        }
    }

    /// Undo the edits staged since `checkpoint`; spawners added after it are dropped.
    // hidden: used only by generated modify_custom_spawners dispatch
    #[doc(hidden)]
    pub fn rollback(&mut self, checkpoint: CustomSpawnersCheckpoint) {
        self.order = checkpoint.order;
        self.added.truncate(checkpoint.added);
    }

    /// Apply the staged order to the level's list: instances the order no longer holds are
    /// dropped (under their mod's containment), and the added ones join.
    // hidden: the game applies the edits once every listener ran
    #[doc(hidden)]
    pub fn commit(self, list: &mut CustomSpawners) {
        let Self {
            order, mut added, ..
        } = self;
        let mut old: Vec<Option<SpawnerSlot>> = list.entries.drain(..).map(Some).collect();
        list.entries = order
            .into_iter()
            .filter_map(|place| match place {
                Staged::Existing(index) => old[index].take(),
                Staged::Added(index) => added[index].take().map(|s| SpawnerSlot::Mod(Some(s))),
            })
            .collect();
    }
}

impl fmt::Debug for CustomSpawners {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut list = f.debug_list();
        for slot in &self.entries {
            match slot {
                SpawnerSlot::Builtin(kind) => {
                    list.entry(kind);
                }
                SpawnerSlot::Mod(Some(spawner)) => {
                    list.entry(&format_args!("Mod({})", spawner.owner_id()));
                }
                SpawnerSlot::Mod(None) => {
                    list.entry(&format_args!("Mod(<in-flight>)"));
                }
            }
        }
        list.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };

    struct DropCounter(Arc<AtomicUsize>);
    impl CustomSpawner for DropCounter {
        fn tick(&mut self, _: &mut dyn ServerHost, _: LevelRef, _: bool) {}
    }
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn builtins(list: &CustomSpawners) -> Vec<SpawnerKind> {
        list.builtins().collect()
    }

    #[test]
    fn vanilla_list_keeps_builtin_order_and_empty_starts_empty() {
        let list = CustomSpawners::vanilla();
        assert_eq!(list.len(), 5);
        assert!(!list.is_empty());
        assert_eq!(
            builtins(&list),
            [
                SpawnerKind::WanderingTrader,
                SpawnerKind::Patrol,
                SpawnerKind::Cat,
                SpawnerKind::VillageSiege,
                SpawnerKind::Phantom,
            ]
        );
        let empty = CustomSpawners::empty();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert_eq!(empty.builtins().count(), 0);
    }

    #[test]
    fn removing_takes_a_builtin_out_once() {
        let mut list = CustomSpawners::vanilla();
        let mut editor = CustomSpawnersEditor::new(&list);
        assert!(editor.contains(SpawnerKind::Cat));
        assert!(editor.remove(SpawnerKind::Cat));
        assert!(!editor.contains(SpawnerKind::Cat));
        assert!(!editor.remove(SpawnerKind::Cat));
        editor.commit(&mut list);
        assert_eq!(list.len(), 4);
        assert!(!list.contains(SpawnerKind::Cat));
    }

    #[test]
    fn a_spawner_can_go_before_a_builtin_and_an_entry_can_move() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut list = CustomSpawners::vanilla();
        let mut editor = CustomSpawnersEditor::new(&list);
        assert!(editor.insert(0, DropCounter(drops.clone())));
        assert!(!editor.insert(99, DropCounter(drops.clone())));
        assert_eq!(
            drops.load(Ordering::Relaxed),
            1,
            "a refused spawner is dropped"
        );
        assert!(matches!(editor.get(0), Some(SpawnerEntry::Mod { .. })));
        assert_eq!(editor.position(SpawnerKind::WanderingTrader), Some(1));
        assert!(editor.move_entry(0, 5));
        assert_eq!(editor.position(SpawnerKind::WanderingTrader), Some(0));
        assert!(matches!(editor.get(5), Some(SpawnerEntry::Mod { .. })));
        assert!(!editor.move_entry(0, 6));
        editor.commit(&mut list);
        assert_eq!(list.len(), 6);
        assert!(matches!(list.take_at(5), Some(CustomSpawnerStep::Mod(_))));
    }

    #[test]
    fn rollback_undoes_a_listeners_edits_and_drops_what_it_added() {
        let kept_drops = Arc::new(AtomicUsize::new(0));
        let rolled_back_drops = Arc::new(AtomicUsize::new(0));
        let mut list = CustomSpawners::vanilla();
        let mut editor = CustomSpawnersEditor::new(&list);
        editor.add(DropCounter(kept_drops.clone()));
        let checkpoint = editor.checkpoint();
        assert!(editor.remove(SpawnerKind::Patrol));
        assert!(editor.remove(SpawnerKind::Phantom));
        editor.add(DropCounter(rolled_back_drops.clone()));
        // Five built-ins, plus one, less two, plus one.
        assert_eq!(editor.len(), 5);
        assert_eq!(rolled_back_drops.load(Ordering::Relaxed), 0);

        editor.rollback(checkpoint);
        assert_eq!(rolled_back_drops.load(Ordering::Relaxed), 1);
        assert_eq!(kept_drops.load(Ordering::Relaxed), 0);
        editor.commit(&mut list);
        assert_eq!(list.len(), 6);
        assert_eq!(
            builtins(&list),
            [
                SpawnerKind::WanderingTrader,
                SpawnerKind::Patrol,
                SpawnerKind::Cat,
                SpawnerKind::VillageSiege,
                SpawnerKind::Phantom,
            ]
        );
        drop(list);
        assert_eq!(kept_drops.load(Ordering::Relaxed), 1);
    }

    /// An earlier listener's stateful instance is still in the level, with its state, after a
    /// later listener removed it and then failed.
    #[test]
    fn an_earlier_instance_survives_a_later_failed_removal() {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut list = CustomSpawners::empty();
        let mut editor = CustomSpawnersEditor::new(&list);
        editor.add(DropCounter(drops.clone()));
        editor.commit(&mut list);
        assert_eq!(list.len(), 1);

        let mut editor = CustomSpawnersEditor::new(&list);
        let checkpoint = editor.checkpoint();
        assert!(editor.remove_at(0));
        assert!(editor.is_empty());
        editor.rollback(checkpoint);
        editor.commit(&mut list);
        assert_eq!(list.len(), 1);
        assert_eq!(drops.load(Ordering::Relaxed), 0, "nothing was dropped");
        assert!(matches!(list.take_at(0), Some(CustomSpawnerStep::Mod(_))));

        // A removal no listener undid drops the instance once the edits are applied. (The
        // instance taken above was dropped there: this one counts on its own.)
        let drops = Arc::new(AtomicUsize::new(0));
        let mut list = CustomSpawners::empty();
        let mut editor = CustomSpawnersEditor::new(&list);
        editor.add(DropCounter(drops.clone()));
        editor.commit(&mut list);
        let mut editor = CustomSpawnersEditor::new(&list);
        assert!(editor.remove_at(0));
        assert_eq!(drops.load(Ordering::Relaxed), 0, "still staged");
        editor.commit(&mut list);
        assert!(list.is_empty());
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn take_at_and_put_back_skip_reentered_slots() {
        let dropped = Arc::new(AtomicBool::new(false));
        struct Tracked(Arc<AtomicBool>);
        impl CustomSpawner for Tracked {
            fn tick(&mut self, _: &mut dyn ServerHost, _: LevelRef, _: bool) {}
        }
        impl Drop for Tracked {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
            }
        }

        let mut list = CustomSpawners::empty();
        let mut editor = CustomSpawnersEditor::new(&list);
        editor.add(Tracked(dropped.clone()));
        editor.commit(&mut list);
        let CustomSpawnerStep::Mod(spawner) = list.take_at(0).unwrap() else {
            panic!("expected mod spawner at slot 0");
        };
        assert!(matches!(list.take_at(0), Some(CustomSpawnerStep::Skip)));
        list.put_back(0, spawner);
        assert!(!dropped.load(Ordering::Relaxed));
        drop(list);
        assert!(dropped.load(Ordering::Relaxed));
    }
}
