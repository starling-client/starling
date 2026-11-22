//! The installed mods, who is subscribed to what, the crash marks around a call, and the
//! unified hook-dispatch failure rule.
//!
//! Generated dispatch uses [`invoke`] to distinguish a completed callback, a phase refusal and
//! a panic. A panic disables that mod's callback for that hook for the session, reports the mod
//! and hook in the error log, and queues a local client error toast. A headless dedicated server
//! logs the error without presenting a toast. A phase refusal is reported each time and does not
//! disable the callback. Startup [`invoke_registration`] uses the same panic boundary for the
//! once-only `Hooks::register` call.
//!
//! Dispatch and the owning publication boundary discard unfinished work. Later eligible listeners
//! still run from the last accepted outputs; only a completed proposal can cancel or replace.
//! Output validation reports each refusal without disabling the callback. A chunk-disk panic or
//! codec error fails publication of the job, not the remaining listener calls.
use crate::*;

// no wire: installed hook trait objects and shared dispatch ownership are process-local.
/// One native mod, with its bundle's `[mod] id` and hook implementation.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct ModEntry {
    pub(crate) id: &'static str,
    pub(crate) hooks: Box<dyn Hooks>,
}

impl ModEntry {
    /// Bind a hook implementation to its nonempty bundle id.
    pub fn new(id: &'static str, hooks: Box<dyn Hooks>) -> Self {
        assert!(!id.is_empty(), "native mod id must not be empty");
        Self { id, hooks }
    }

    /// Marks the thread as inside this mod's `hook` until the guard drops, so a panic in it
    /// names both. The generated hook dispatcher calls it.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn enter(&self, hook: &'static str) -> crate::content::crash::ModGuard {
        mark_mod(self.id, hook)
    }

    /// The first client-side hook this mod subscribes to, by name: the game refuses a
    /// `side = "server"` mod that has one.
    pub(crate) fn client_hook(&self) -> Option<&'static str> {
        client_hook_name(self.hooks.subscribed())
    }
}

/// The first client-side hook among `subscribed`, by name, in catalog order.
pub(crate) fn client_hook_name(subscribed: &[Option<HookId>]) -> Option<&'static str> {
    generated::CLIENT_HOOKS
        .iter()
        .find(|(id, _)| {
            subscribed
                .iter()
                .flatten()
                .any(|s| *s as usize == *id as usize)
        })
        .map(|(_, name)| *name)
}

// no wire: installed hook trait objects and shared dispatch ownership are process-local.
/// The ordered list of named mods shared by the client and server.
pub type Mods = Arc<[ModEntry]>;

type NamedPoison =
    std::collections::HashMap<(usize, &'static str), std::collections::BTreeSet<Identifier>>;

pub(crate) struct Installed {
    pub(crate) mods: Mods,
    pub(crate) by_id: std::collections::HashMap<&'static str, usize>,
    pub(crate) subscribers: sunrise_mod_runtime::subscribers::Subscribers,
    pub(crate) disabled: Box<[std::sync::atomic::AtomicBool]>,
    /// One installed owner per mod for delayed tasks, independent of its other callbacks.
    pub(crate) scheduled_tasks: Box<[crate::callback_owner::CallbackOwner]>,
    pub(crate) configuration_task_owners: std::sync::Mutex<crate::configuration_task::TaskOwners>,
    /// Retained submission owners are installed once per subscribed mod, one per overlay.
    pub(crate) overlay_owners: std::collections::HashMap<
        &'static str,
        [crate::callback_owner::CallbackOwner; BlockScreenEffect::COUNT],
    >,
    mod_id_owners: std::collections::HashMap<&'static str, usize>,
    content_owners: OnceLock<std::collections::HashMap<Box<str>, Option<usize>>>,
    disabled_count: std::sync::atomic::AtomicUsize,
    /// Per-hook sets of mods whose listener panicked and is disabled for that
    /// hook for the rest of the session. Filled only on a panic; `poison_free`
    /// answers through one atomic load, so a dispatch with no poison pays nothing.
    poisons: std::sync::Mutex<Option<Box<[Option<std::collections::HashSet<&'static str>>]>>>,
    poison_free: std::sync::atomic::AtomicBool,
    /// Session poison for named callbacks rebuilt with resources or a connection.
    named_poison: std::sync::Mutex<NamedPoison>,
    retained_poisons: RetainedPoisons,
    /// Hook-listener panic reports queued for this installed set's client error toast.
    hook_panics: std::sync::Mutex<Vec<HookPanicReport>>,
    has_hook_panics: std::sync::atomic::AtomicBool,
}

impl Installed {
    pub(crate) fn new(mods: Mods) -> Self {
        // Each hook's subscribers are ordered once here: by the priority their `#[listener]`
        // gives, then in dependency order.
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
        let subscribers = sunrise_mod_runtime::subscribers::Subscribers::new(
            generated::HOOK_COUNT,
            mods.len(),
            subscriptions,
        );
        let overlay_owners = subscribers
            .indices(HookId::ExtractBlockScreenEffect as usize)
            .map(|index| {
                let id = mods[index].id;
                (
                    id,
                    std::array::from_fn(|_| {
                        crate::callback_owner::CallbackOwner::for_index(id, index)
                    }),
                )
            })
            .collect();
        let disabled = (0..mods.len())
            .map(|_| std::sync::atomic::AtomicBool::new(false))
            .collect();
        let scheduled_tasks = mods
            .iter()
            .enumerate()
            .map(|(index, entry)| crate::callback_owner::CallbackOwner::for_index(entry.id, index))
            .collect();
        let mod_id_owners = mods
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.id, index))
            .collect();
        Self {
            by_id: mods
                .iter()
                .enumerate()
                .map(|(i, entry)| (entry.id, i))
                .collect(),
            mods,
            subscribers,
            disabled,
            scheduled_tasks,
            configuration_task_owners: Default::default(),
            overlay_owners,
            mod_id_owners,
            content_owners: OnceLock::new(),
            disabled_count: std::sync::atomic::AtomicUsize::new(0),
            poisons: std::sync::Mutex::new(None),
            poison_free: std::sync::atomic::AtomicBool::new(true),
            named_poison: Default::default(),
            retained_poisons: RetainedPoisons::default(),
            hook_panics: std::sync::Mutex::new(Vec::new()),
            has_hook_panics: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

pub(crate) static MODS: OnceLock<Installed> = OnceLock::new();

#[cfg(any(test, feature = "testing"))]
thread_local! {
    pub(crate) static TEST_MODS: std::cell::Cell<Option<&'static Installed>> = const { std::cell::Cell::new(None) };
}

pub(crate) fn installed() -> Option<&'static Installed> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(installed) = TEST_MODS.get() {
        return Some(installed);
    }
    MODS.get()
}

use crate::__private::HookPanicReport;

thread_local! {
    static HOOK_DISPATCH_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Active hook-dispatch panic-containment nesting depth on this thread.
pub(crate) fn hook_dispatch_depth() -> u32 {
    HOOK_DISPATCH_DEPTH.get()
}

struct DispatchPanicScope(u32);

impl DispatchPanicScope {
    #[inline]
    fn enter() -> Self {
        let prev = HOOK_DISPATCH_DEPTH.get();
        HOOK_DISPATCH_DEPTH.set(prev.saturating_add(1));
        Self(prev)
    }
}

impl Drop for DispatchPanicScope {
    #[inline]
    fn drop(&mut self) {
        HOOK_DISPATCH_DEPTH.set(self.0);
    }
}

pub(crate) fn take_hook_panics() -> Vec<HookPanicReport> {
    let Some(installed) = installed() else {
        return Vec::new();
    };
    if !installed
        .has_hook_panics
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Vec::new();
    }
    let mut queue = installed
        .hook_panics
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    installed
        .has_hook_panics
        .swap(false, std::sync::atomic::Ordering::AcqRel);
    std::mem::take(&mut *queue)
}

/// Install the process-wide ordered list of mods.
pub(crate) fn install(mods: Mods) -> &'static Installed {
    let installed = Installed::new(mods);
    #[cfg(any(test, feature = "testing"))]
    {
        // Tests can replace the list; mods() keeps its static slice lifetime.
        let installed = Box::leak(Box::new(installed));
        TEST_MODS.set(Some(installed));
        installed
    }
    #[cfg(not(any(test, feature = "testing")))]
    {
        assert!(MODS.set(installed).is_ok(), "trait mods are installed once");
        MODS.get().unwrap()
    }
}

/// Checkpoint for an owning action around isolated dispatch. A trapped listener
/// resets its outputs, but the owner must also abort any tentative Host writes.
/// Unrelated threads do not affect the checkpoint.
// hidden: the game's own decide/fire/apply actions read it; it is not a mod-facing call.
#[doc(hidden)]
pub fn isolated_failures() -> u64 {
    LISTENER_SERIAL.get().1
}

/// Dispatch keys include engine-invoked callbacks that are not themselves catalog hooks.
#[derive(Clone, Copy)]
pub(crate) enum Callback {
    Hook(generated::HookId),
    DamageReduction,
    Registration,
}

impl From<generated::HookId> for Callback {
    fn from(hook: generated::HookId) -> Self {
        Self::Hook(hook)
    }
}

impl Callback {
    fn index(self) -> usize {
        match self {
            Self::Hook(hook) => hook as usize,
            Self::DamageReduction => generated::HOOK_COUNT,
            Self::Registration => generated::HOOK_COUNT + 1,
        }
    }
}

/// Mark `id`'s listener of `hook` disabled for the rest of the session: it is skipped by
/// that one hook alone — its other hooks, Parts, and every other mod's listeners are
/// untouched, unlike a whole-mod disable. Returns `true` only on the first panic for
/// `(hook, id)`, so the error log line and error toast fire once.
fn poison(callback: Callback, id: &'static str) -> bool {
    let (entered, trapped) = LISTENER_SERIAL.get();
    LISTENER_SERIAL.set((entered, trapped.wrapping_add(1)));
    let Some(installed) = installed() else {
        return true;
    };
    let mut poisons = installed.poisons.lock().unwrap_or_else(|e| e.into_inner());
    let sets = poisons
        .get_or_insert_with(|| std::vec![None; generated::HOOK_COUNT + 2].into_boxed_slice());
    let first = sets[callback.index()]
        .get_or_insert_with(std::collections::HashSet::new)
        .insert(id);
    installed
        .poison_free
        .store(false, std::sync::atomic::Ordering::Release);
    first
}

fn is_poisoned(installed: &Installed, callback: impl Into<Callback>, id: &str) -> bool {
    // One atomic load on the usual path: the sets only fill on a panic.
    !installed
        .poison_free
        .load(std::sync::atomic::Ordering::Acquire)
        && installed
            .poisons
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|poisons| poisons[callback.into().index()].as_ref())
            .is_some_and(|poisoned| poisoned.contains(id))
}

/// Whether `id`'s listener of `hook` is poisoned for the session.
pub(crate) fn poisoned(hook: impl Into<Callback>, id: &str) -> bool {
    installed().is_some_and(|installed| is_poisoned(installed, hook, id))
}

/// Record a mod callback panic under the unified dispatch failure rule.
///
/// On the first panic for `(hook_id, mod_id)`, disables that mod's callback for that hook for the
/// rest of the session (leaving other hooks and other mods running), logs an `error!` line naming
/// the mod, hook, and panic message, and queues one [`HookPanicReport`] for the client's error
/// toast.
fn record_hook_panic(
    callback: Callback,
    hook: &'static str,
    mod_id: &'static str,
    payload: &(dyn std::any::Any + Send),
) {
    if poison(callback, mod_id) {
        log_and_queue_panic(hook, mod_id, payload);
    }
}

pub(crate) fn log_and_queue_panic(
    hook: &'static str,
    mod_id: &'static str,
    payload: &(dyn std::any::Any + Send),
) {
    log_and_queue_named_panic(installed(), hook, mod_id, None, payload);
}

pub(crate) fn log_and_queue_named_panic(
    installed: Option<&Installed>,
    hook: &'static str,
    mod_id: &'static str,
    named_callback: Option<&Identifier>,
    payload: &(dyn std::any::Any + Send),
) {
    let reason = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("panic");
    if let Some(id) = named_callback {
        log::error!(
            "mod {mod_id} panicked in {hook} ({id}) and has been disabled for this callback for the rest of the session: {reason}"
        );
    } else {
        log::error!(
            "mod {mod_id} panicked in hook {hook} and has been disabled for this hook for the rest of the session: {reason}"
        );
    }
    if let Some(installed) = installed {
        let mut queue = installed
            .hook_panics
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        queue.push(HookPanicReport { mod_id, hook });
        installed
            .has_hook_panics
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

/// Runs one listener. Only an unwind disables it; returned errors remain ordinary results.
#[inline]
pub(crate) fn invoke<T>(
    hook_id: impl Into<Callback>,
    hook: &'static str,
    mod_id: &'static str,
    call: impl FnOnce() -> T,
) -> Option<T> {
    contain_callback(call, |payload| {
        record_hook_panic(hook_id.into(), hook, mod_id, payload)
    })
}

#[cold]
pub(crate) fn report_output_refusal(mod_id: &str, hook: &str, field: &str) {
    log::warn!(
        "mod {mod_id} hook {hook} output {field} refused: its outputs are dropped and the preceding ones stand"
    );
}

/// Registered codecs use the diagnostic fence independently of gameplay hook switches.
/// Each invocation site owns its own panic disablement.
pub(crate) fn invoke_registered_codec<T>(
    mod_id: &'static str,
    operation: &'static str,
    poisoned: &std::sync::atomic::AtomicBool,
    call: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    use std::sync::atomic::Ordering;
    if poisoned.load(Ordering::Acquire) {
        return Err(format!("mod {mod_id}: {operation} disabled after a panic"));
    }
    let _owner = mark_mod(mod_id, operation);
    contain_callback(call, |payload| {
        if !poisoned.swap(true, Ordering::AcqRel) {
            log_and_queue_panic(operation, mod_id, payload);
        }
    })
    .ok_or_else(|| format!("mod {mod_id}: {operation} panicked"))?
    .map_err(|reason| {
        let reason = format!("mod {mod_id}: {operation} refused: {reason}");
        log::warn!("{reason}");
        reason
    })
}

/// Run a callback with the crash hook deferred to its owning diagnostic policy.
#[inline]
pub(crate) fn contain_callback<T>(
    call: impl FnOnce() -> T,
    on_panic: impl FnOnce(&(dyn std::any::Any + Send)),
) -> Option<T> {
    let _scope = DispatchPanicScope::enter();
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)) {
        Ok(value) => Some(value),
        Err(payload) => {
            on_panic(&*payload);
            dispose_panic_payload(payload);
            None
        }
    }
}

/// Check the session's enablement for a callback identified across reloads or peers.
// hidden: the game owns the session’s callback enablement and diagnostics.
#[doc(hidden)]
pub fn session_callback_enabled(
    hook: generated::HookId,
    owner: &'static str,
    id: &Identifier,
) -> bool {
    installed().is_some_and(|installed| {
        installed
            .by_id
            .get(owner)
            .is_some_and(|&index| mod_index_on(index))
            && !installed
                .named_poison
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&(hook as usize, owner))
                .is_some_and(|poisoned| poisoned.contains(id))
    })
}

/// Invoke an enabled named callback through the same failure boundary as a hook subscriber.
// hidden: the game owns the session’s callback enablement and diagnostics.
#[doc(hidden)]
pub fn call_session_callback<T>(
    hook: generated::HookId,
    owner: &'static str,
    id: &Identifier,
    operation: &'static str,
    call: impl FnOnce() -> T,
) -> Result<T, String> {
    let _owner = mark_mod(owner, operation);
    contain_callback(call, |payload| {
        let (entered, trapped) = LISTENER_SERIAL.get();
        LISTENER_SERIAL.set((entered, trapped.wrapping_add(1)));
        let first = installed().is_none_or(|installed| {
            installed
                .named_poison
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry((hook as usize, owner))
                .or_default()
                .insert(id.clone())
        });
        if first {
            log_and_queue_named_panic(installed(), operation, owner, Some(id), payload);
        }
    })
    .ok_or_else(|| format!("mod {owner}: {operation} ({id}) panicked"))
}

/// Retained wrappers may delegate engine paint. The engine's own panic keeps its usual policy.
pub(crate) fn without_callback_scope<T>(call: impl FnOnce() -> T) -> T {
    let _scope = DispatchPanicScope(HOOK_DISPATCH_DEPTH.replace(0));
    call()
}

#[path = "runtime/retained.rs"]
pub(crate) mod retained;
pub(crate) use retained::RetainedCallback;
use retained::RetainedPoisons;

// A mod's panic payload may itself panic on drop. Stay in DispatchPanicScope and do not
// recursively destroy a secondary payload that may repeat the same panic.
#[cold]
fn dispose_panic_payload(payload: Box<dyn std::any::Any + Send>) {
    if let Err(secondary) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(payload)))
    {
        std::mem::forget(secondary);
    }
}

/// Drop one mod-owned registration value after deciding whether to publish its batch. Never
/// let one destructor unwind across other entries or the next listener, and keep a replaced
/// value attributed to its original mod rather than to the mod that replaced it.
pub fn discard_registration<T>(
    (hook_id, hook): (generated::HookId, &'static str),
    owner: Option<&'static str>,
    value: T,
) {
    let current = crate::content::crash::current_native_mod();
    let _owner = owner
        .filter(|&id| Some(id) != current)
        .map(|id| mark_mod(id, hook));
    invoke(hook_id, hook, owner.unwrap_or("unknown"), || drop(value));
}

/// A single listener's keyed edits, in order of first insertion. A repeat replaces only
/// a pending value, so its destructor still runs inside that listener's invocation fence.
pub(crate) struct RegistrationEdits<T, K = String>(std::collections::BTreeMap<K, (usize, T)>);

impl<T, K> Default for RegistrationEdits<T, K> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T, K: Ord> RegistrationEdits<T, K> {
    pub(crate) fn insert(&mut self, key: K, value: T) {
        use std::collections::btree_map::Entry;
        let index = self.0.len();
        match self.0.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert((index, value));
            }
            Entry::Occupied(mut entry) => drop(std::mem::replace(&mut entry.get_mut().1, value)),
        }
    }

    pub(crate) fn ordered(self) -> impl Iterator<Item = (K, T)> {
        let mut rows: Vec<_> = self.0.into_iter().collect();
        rows.sort_unstable_by_key(|(_, (index, _))| *index);
        rows.into_iter().map(|(key, (_, value))| (key, value))
    }
}

/// Last registration wins. Keep the declaring owner until publication is complete so retiring
/// an earlier value can run its destructor under the appropriate mod's scope.
pub(crate) struct RegistrationMap<T, K = String>(
    std::collections::BTreeMap<K, (Option<&'static str>, T)>,
);

impl<T, K> Default for RegistrationMap<T, K> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<T, K: Ord> RegistrationMap<T, K> {
    pub(crate) fn finish(
        &mut self,
        site: (generated::HookId, &'static str),
        publish: bool,
        edits: impl IntoIterator<Item = (K, T)>,
    ) {
        let owner = crate::content::crash::current_native_mod();
        if publish {
            let retired: Vec<_> = edits
                .into_iter()
                .filter_map(|(key, value)| self.0.insert(key, (owner, value)))
                .collect();
            for (owner, value) in retired {
                discard_registration(site, owner, value);
            }
        } else {
            for (_, value) in edits {
                discard_registration(site, owner, value);
            }
        }
    }

    pub(crate) fn into_values(self) -> std::collections::BTreeMap<K, T> {
        self.0
            .into_iter()
            .map(|(key, (_, value))| (key, value))
            .collect()
    }
}

/// Run one mod's startup `Hooks::register` callback under the unified hook-dispatch failure rule.
///
/// Returns `true` if registration completed without panicking, or `false` after recording the
/// failure, logging the error, and queueing a client error toast so the caller can roll back the
/// mod's partial registrations and continue with remaining mods.
pub(crate) fn invoke_registration(mod_id: &'static str, call: impl FnOnce()) -> bool {
    // `register` is not a generated event; reserve one private slot in the same table.
    if installed().is_some_and(|installed| is_poisoned(installed, Callback::Registration, mod_id)) {
        return false;
    }
    contain_registration(mod_id, call)
}

/// Cleanup must run even after the panicking callback has already been disabled.
pub(crate) fn discard_startup_registrations<T>(
    mod_id: &'static str,
    values: impl IntoIterator<Item = T>,
) {
    for value in values {
        contain_registration(mod_id, || drop(value));
    }
}

fn contain_registration(mod_id: &'static str, call: impl FnOnce()) -> bool {
    contain_callback(call, |payload| {
        if poison(Callback::Registration, mod_id) {
            log_and_queue_panic("register", mod_id, payload);
        }
    })
    .is_some()
}

/// The process-wide list in dispatch order, or an empty list before installation.
pub(crate) fn mods() -> &'static [ModEntry] {
    installed().map_or(&[], |installed| &installed.mods)
}

pub(crate) fn listened(hook: generated::HookId) -> bool {
    installed().is_some_and(|installed| installed.subscribers.contains_any(hook as usize))
}

pub(crate) fn enabled_listened(hook: generated::HookId) -> bool {
    installed().is_some_and(|installed| {
        installed.subscribers.indices(hook as usize).any(|index| {
            !installed.disabled[index].load(std::sync::atomic::Ordering::Relaxed)
                && !is_poisoned(installed, hook, installed.mods[index].id)
        })
    })
}

/// Switch an installed trait mod's subsequent dispatches on or off.
pub(crate) fn set_mod_enabled(id: &str, on: bool) -> bool {
    let Some(installed) = installed() else {
        return false;
    };
    let mut found = false;
    for (entry, disabled) in installed.mods.iter().zip(&*installed.disabled) {
        if entry.id == id {
            // Publish the slow path before disabling. A repeated disable reserves and releases
            // its own count; enabling releases the count of the previously disabled entry.
            // This keeps concurrent toggles from publishing a stale false flag.
            if !on {
                installed
                    .disabled_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            let was_disabled = disabled.swap(!on, std::sync::atomic::Ordering::SeqCst);
            if was_disabled {
                installed
                    .disabled_count
                    .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
            if was_disabled == on {
                MODS_SWITCHED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if !on {
                    sunrise_mod_runtime::water_tickets::remove_owner(id);
                }
            }
            found = true;
        }
    }
    found
}

/// Cached at load/toggle time, so the scheduler need not inspect content on its normal path.
pub(crate) fn any_mod_disabled() -> bool {
    installed().is_some_and(|installed| {
        installed
            .disabled_count
            .load(std::sync::atomic::Ordering::SeqCst)
            != 0
    })
}

/// How many times a mod was switched on or off: what a kept capability compares to tell that a
/// provider may have come or gone (`BlockCapabilityCache`).
static MODS_SWITCHED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The count of mod switches so far.
pub(crate) fn mods_switched() -> u64 {
    MODS_SWITCHED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether an installed mod subscribes to an injected hook.
// hidden: a game-side helper, not part of the mod surface
#[doc(hidden)]
pub fn subscribed_by(index: usize, hook: Option<generated::HookId>) -> bool {
    hook.is_some_and(|hook| {
        installed().is_some_and(|installed| {
            index < installed.mods.len()
                && !installed.disabled[index].load(std::sync::atomic::Ordering::Relaxed)
                && installed.subscribers.contains(hook as usize, index)
        })
    })
}

pub(crate) fn mod_index_on(index: usize) -> bool {
    installed().is_none_or(|installed| {
        installed
            .disabled
            .get(index)
            .is_none_or(|disabled| !disabled.load(std::sync::atomic::Ordering::Relaxed))
    })
}

/// Freeze primary IDs first, then the first admitted owner of each alias.
/// An admitted content-only owner reserves its aliases without a dispatch slot.
pub(crate) fn install_content_owners<'a>(owners: impl IntoIterator<Item = (&'a str, &'a str)>) {
    if let Some(installed) = installed() {
        installed.content_owners.get_or_init(|| {
            let mut by_namespace: std::collections::HashMap<Box<str>, _> = installed
                .mod_id_owners
                .iter()
                .map(|(&id, &index)| (id.into(), Some(index)))
                .collect();
            for (namespace, owner) in owners {
                by_namespace
                    .entry(namespace.into())
                    .or_insert_with(|| installed.mod_id_owners.get(owner).copied());
            }
            by_namespace
        });
    }
}

pub(crate) fn content_owners_published() -> bool {
    installed().is_none_or(|installed| installed.content_owners.get().is_some())
}

/// Content ownership is resolved at startup, not by scanning mods on a tick.
pub(crate) fn content_owner_disabled(id: &str) -> bool {
    any_mod_disabled() && content_owner_index(id).is_some_and(|index| !mod_index_on(index))
}

pub(crate) fn content_owner_index(id: &str) -> Option<usize> {
    let (namespace, _) = id.split_once(':')?;
    let installed = installed()?;
    match installed.content_owners.get() {
        Some(owners) => owners.get(namespace).copied().flatten(),
        None => installed.mod_id_owners.get(namespace).copied(),
    }
}

/// `mods` in order, each marked as running `hook` until the next one is (or the loop ends), so a
/// panic in it names the mod. The generated dispatch loops over it.
pub fn entered_registration<'a>(
    mods: &'a [ModEntry],
    hook: &'static str,
) -> impl Iterator<Item = &'a dyn Hooks> + 'a {
    Entered {
        mods: mods.iter(),
        hook,
        guard: None,
        scope: None,
    }
}

/// Enabled subscribers in dispatch order, each paired with its `[mod] id` and marked as running
/// `hook` until the next one is (or the loop ends), so a panic in it names both.
pub(crate) fn entered_with_id(
    hook_id: generated::HookId,
    hook: &'static str,
) -> impl Iterator<Item = (&'static str, &'static dyn Hooks)> {
    entered_with_id_matching(hook_id, hook, |_| true)
}

/// [`entered_with_id`] filtered by `allow`, skipping rejected subscribers before entering their
/// crash scope or advancing the listener serial.
pub(crate) fn entered_with_id_matching<'a>(
    hook_id: generated::HookId,
    hook: &'static str,
    allow: impl FnMut(&str) -> bool + 'a,
) -> impl Iterator<Item = (&'static str, &'static dyn Hooks)> + 'a {
    entered_subscribers_matching(hook_id, hook, allow).map(|entry| (entry.id, &*entry.hooks))
}

/// How many listeners of `hook` are poisoned for the session.
pub(crate) fn poisoned_count(hook: generated::HookId) -> usize {
    installed().map_or(0, |installed| {
        if installed
            .poison_free
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return 0;
        }
        installed
            .poisons
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|poisons| poisons[hook as usize].as_ref())
            .map_or(0, |poisoned| poisoned.len())
    })
}

fn entered_subscribers_matching<'a>(
    hook_id: generated::HookId,
    hook: &'static str,
    allow: impl FnMut(&str) -> bool + 'a,
) -> impl Iterator<Item = &'static ModEntry> + 'a {
    let mut crash_guard = crate::dispatch_scope::ModGuard::empty();
    let scope = crate::dispatch_scope::snapshot();
    listening_matching(hook_id, allow).map(move |(_, entry)| {
        scope.replace(
            &mut crash_guard,
            crate::content::crash::ModCall {
                id: entry.id,
                hook,
                part: None,
            },
        );
        entry
    })
}

pub(crate) fn mark_mod(id: &'static str, hook: &'static str) -> crate::content::crash::ModGuard {
    crate::content::crash::enter_native_mod(crate::content::crash::ModCall {
        id,
        hook,
        part: None,
    })
}

/// The parts in `parts`, each paired with its mod id and marked as running `hook` with its
/// type, so a panic in it names the mod, the hook and the part.
pub(crate) fn entered_parts<'a, P: ?Sized>(
    hook_id: generated::HookId,
    parts: &'a [Attached<P>],
    hook: &'static str,
) -> impl Iterator<Item = (&'static str, &'a P)> {
    let mut guard = crate::dispatch_scope::ModGuard::empty();
    let scope = crate::dispatch_scope::snapshot();
    parts
        .iter()
        .filter(move |attached| {
            mod_index_on(attached.mod_index) && !poisoned(hook_id, attached.mod_id)
        })
        .map(move |attached| {
            // The last part's mark goes first, as in `Entered`.
            scope.replace(
                &mut guard,
                crate::content::crash::ModCall {
                    id: attached.mod_id,
                    hook,
                    part: Some(attached.type_name),
                },
            );
            (attached.mod_id, &*attached.part)
        })
}

pub(crate) struct Entered<'a> {
    pub(crate) mods: std::slice::Iter<'a, ModEntry>,
    pub(crate) hook: &'static str,
    pub(crate) guard: Option<crate::content::crash::ModGuard>,
    scope: Option<crate::dispatch_scope::Scope>,
}

impl<'a> Iterator for Entered<'a> {
    type Item = &'a dyn Hooks;

    fn next(&mut self) -> Option<Self::Item> {
        // The last mod's mark goes first, so the next one's puts back what was there before.
        drop(self.guard.take());
        let entry = self.mods.next()?;
        self.guard = Some(
            self.scope
                .get_or_insert_with(crate::dispatch_scope::snapshot)
                .enter(crate::content::crash::ModCall {
                    id: entry.id,
                    hook: self.hook,
                    part: None,
                }),
        );
        Some(&*entry.hooks)
    }
}

/// Whether a mod claims a cancellable hook.
///
/// A mod still told once an earlier one cancelled marks its method `#[listener(receive_cancelled)]`
/// and reads the event's `cancelled` field, which it may set back to `false`:
///
/// ```
/// use mod_api::{ClientHost, Flow, Hooks, RenderGuiPreEvent};
/// #[derive(Default)]
/// struct Gui;
/// #[mod_api::sunrise_mod]
/// impl Hooks for Gui {
///     #[listener(priority = -10, receive_cancelled)]
///     fn render_gui_pre(&self, _: &dyn ClientHost, event: &mut RenderGuiPreEvent) -> Flow {
///         event.cancelled = false;
///         Flow::Continue
///     }
/// }
/// ```
///
/// A hook no mod can cancel has no cancel to receive, so the option doesn't compile there:
///
/// ```compile_fail
/// use mod_api::{ClientHost, ClientTickPreEvent, Hooks};
/// #[derive(Default)]
/// struct Ticker;
/// #[mod_api::sunrise_mod]
/// impl Hooks for Ticker {
///     #[listener(receive_cancelled)]
///     fn client_tick_pre(&self, _: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {}
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum Flow {
    /// Go on to the next listener and then the game's own behaviour.
    Continue,
    /// Stop here; the game's own behaviour does not run.
    Cancel,
}

/// Whether a subscriber of a cancel hook is still told once the event is cancelled: the generated
/// dispatch of an event the game hands over cancelled returns early without one.
pub(crate) fn receives_cancelled(hook: generated::HookId) -> bool {
    installed().is_some_and(|installed| installed.subscribers.receives_cancelled(hook as usize))
}

/// A hook's enabled, unpoisoned subscribers in dispatch order, each with whether it is still told
/// once the event is cancelled. Unlike [`entered_with_id`], nothing is marked: the dispatch enters
/// only the mods it calls, through [`ListenerScope::enter`].
pub(crate) fn listening(
    hook_id: generated::HookId,
) -> impl Iterator<Item = (bool, &'static ModEntry)> {
    listening_matching(hook_id, |_| true)
}

fn listening_matching<'a>(
    hook_id: generated::HookId,
    mut allow: impl FnMut(&str) -> bool + 'a,
) -> impl Iterator<Item = (bool, &'static ModEntry)> + 'a {
    let installed = installed();
    let mut subscribers = installed
        .into_iter()
        .flat_map(move |mods| mods.subscribers.subscribers(hook_id as usize));
    std::iter::from_fn(move || {
        let installed = installed?;
        for subscriber in subscribers.by_ref() {
            let entry = &installed.mods[subscriber.index];
            if installed.disabled[subscriber.index].load(std::sync::atomic::Ordering::Relaxed)
                || is_poisoned(installed, hook_id, entry.id)
                || !allow(entry.id)
            {
                continue;
            }
            bump_listener_serial();
            return Some((subscriber.receives_cancelled, entry));
        }
        None
    })
}

/// Look up the enabled, unpoisoned subscriber owning a namespaced kind without walking the hook's
/// listeners.
pub(crate) fn listening_kind_owner(hook: HookId, kind: &str) -> Option<&'static ModEntry> {
    let (namespace, _) = kind.split_once(':')?;
    let installed = installed()?;
    let &index = installed.by_id.get(namespace)?;
    if !installed.subscribers.contains(hook as usize, index)
        || installed.disabled[index].load(std::sync::atomic::Ordering::Relaxed)
        || is_poisoned(installed, hook, installed.mods[index].id)
    {
        return None;
    }
    bump_listener_serial();
    Some(&installed.mods[index])
}

/// Marks the thread as inside `entry`'s `hook` until the guard drops, and hands back its hooks.
/// One reusable instrumentation slot for loops that filter cancellation or ownership.
pub(crate) struct ListenerScope {
    scope: crate::dispatch_scope::Scope,
    guard: crate::dispatch_scope::ModGuard,
}
impl ListenerScope {
    pub(crate) fn new() -> Self {
        Self {
            scope: crate::dispatch_scope::snapshot(),
            guard: crate::dispatch_scope::ModGuard::empty(),
        }
    }
    #[inline]
    pub(crate) fn enter(
        &mut self,
        entry: &'static ModEntry,
        hook: &'static str,
    ) -> &'static dyn Hooks {
        self.scope.replace(
            &mut self.guard,
            crate::content::crash::ModCall {
                id: entry.id,
                hook,
                part: None,
            },
        );
        &*entry.hooks
    }
}

thread_local! {
    /// Listener entries (including nested hooks) and poisoned traps on this thread.
    static LISTENER_SERIAL: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
}

#[inline]
fn bump_listener_serial() {
    let (entered, trapped) = LISTENER_SERIAL.get();
    LISTENER_SERIAL.set((entered.wrapping_add(1), trapped));
}

pub(crate) fn listener_serial() -> u64 {
    LISTENER_SERIAL.get().0
}

#[cfg(test)]
mod listener_lease_tests {
    use super::*;
    use sunrise_mod_runtime::pending_children::{Decision, Error, PendingChildren};

    #[test]
    fn entering_a_different_hook_expires_a_retained_pending_child() {
        struct Listener;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Listener {
            fn mob_split(
                &self,
                _: &mut dyn crate::ServerHost,
                _: &mut crate::MobSplitEvent<'_>,
            ) -> Flow {
                Flow::Continue
            }
            fn baby_entity_spawn(
                &self,
                _: &mut dyn crate::ServerHost,
                _: &mut crate::BabyEntitySpawnEvent<'_>,
            ) -> Flow {
                Flow::Continue
            }
        }
        install(vec![ModEntry::new("lease-test", Box::new(Listener))].into());
        let mut arena = PendingChildren::default();
        arena
            .run(
                1,
                1,
                vec![7u32],
                |arena, action| {
                    let _outer = listening(generated::HookId::MobSplit).next().unwrap();
                    arena.sync_listener(listener_serial()).unwrap();
                    let child = arena.borrow(action, 0).unwrap();
                    assert_eq!(arena.get(child), Ok(&7));
                    let _nested = listening(generated::HookId::BabyEntitySpawn)
                        .next()
                        .unwrap();
                    arena.sync_listener(listener_serial()).unwrap();
                    assert_eq!(arena.get(child), Err(Error::Expired));
                    let fresh = arena.borrow(action, 0).unwrap();
                    assert_eq!(arena.get(fresh), Ok(&7));
                    Decision::Accept
                },
                |_| true,
            )
            .unwrap();
    }
}

#[cfg(test)]
mod toggle_tests {
    use super::*;

    #[test]
    fn cached_disabled_flag_tracks_repeated_and_independent_toggles() {
        struct Quiet;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Quiet {}
        crate::testing::with_dispatch(
            vec![
                ModEntry::new("a", Box::new(Quiet)),
                ModEntry::new("b", Box::new(Quiet)),
            ]
            .into(),
            || {
                assert!(!any_mod_disabled());
                assert!(set_mod_enabled("a", false));
                assert!(set_mod_enabled("a", false));
                assert!(any_mod_disabled());
                assert!(set_mod_enabled("b", false));
                assert!(set_mod_enabled("a", true));
                assert!(set_mod_enabled("a", true));
                assert!(any_mod_disabled());
                assert!(!set_mod_enabled("missing", true));
                assert!(any_mod_disabled());
                assert!(set_mod_enabled("b", true));
                assert!(!any_mod_disabled());
            },
        );
    }
}
