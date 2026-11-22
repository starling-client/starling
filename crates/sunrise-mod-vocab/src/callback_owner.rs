//! Owner enablement, attribution and panic containment for retained resources.
use std::{
    any::Any,
    cell::{Cell, RefCell},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

thread_local! {
    static CONTAINED: Cell<u32> = const { Cell::new(0) };
    static CAUGHT: RefCell<Option<String>> = const { RefCell::new(None) };
    static RETAINED_CALLBACK: Cell<bool> = const { Cell::new(false) };
    // Keep the original fatal payload in the engine-owned invocation, not in the
    // unwind a wrapper is free to catch. It is taken on exit from the outermost wrapper.
    static DELEGATED_ENGINE_PANIC: RefCell<Option<Box<dyn Any + Send>>> = const { RefCell::new(None) };
}

// The boundary may be monomorphized outside mod_vocab. Keep the thread-local reads and
// writes in non-inline functions so the callback and the game hook share the same slot.
struct PanicScope(Option<String>);

impl PanicScope {
    #[inline(never)]
    fn enter() -> Self {
        CONTAINED.with(|c| c.set(c.get() + 1));
        Self(CAUGHT.with(|c| c.replace(None)))
    }

    #[inline(never)]
    fn take_message(&self) -> Option<String> {
        CAUGHT.with(|c| c.borrow_mut().take())
    }
}

impl Drop for PanicScope {
    #[inline(never)]
    fn drop(&mut self) {
        CONTAINED.with(|c| c.set(c.get() - 1));
        drop(CAUGHT.with(|c| c.replace(self.0.take())));
    }
}

struct SuspendedPanicScope {
    depth: u32,
    caught: Option<String>,
}

impl SuspendedPanicScope {
    #[inline(never)]
    fn enter() -> Self {
        let depth = CONTAINED.with(|c| c.replace(0));
        let caught = CAUGHT.with(|c| c.replace(None));
        Self { depth, caught }
    }
}

impl Drop for SuspendedPanicScope {
    #[inline(never)]
    fn drop(&mut self) {
        CONTAINED.with(|c| c.set(self.depth));
        drop(CAUGHT.with(|c| c.replace(self.caught.take())));
    }
}

/// Catch a real panic and retain its diagnostic, even if dropping its payload also panics.
/// The game panic hook records its attributed message while this boundary is active.
pub fn contain_unwind<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    let scope = PanicScope::enter();
    catch_unwind(AssertUnwindSafe(f)).map_err(|payload| {
        let message = scope
            .take_message()
            .unwrap_or_else(|| panic_payload_text(&*payload));
        // An opaque payload may run mod code when dropped. Never replace the original
        // diagnostic with that destructor's panic, or try to drop a second hostile payload.
        if let Err(secondary) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
            std::mem::forget(secondary);
        }
        message
    })
}

/// Called by the game panic hook to log and record a contained panic instead of a crash.
#[inline(never)]
pub fn record_contained_panic(diagnostic: &dyn Fn() -> String) -> bool {
    if CONTAINED.with(Cell::get) == 0 {
        return false;
    }
    let message = diagnostic();
    CAUGHT.with(|c| *c.borrow_mut() = Some(message));
    true
}

/// The printable message Rust gives string payloads; other payloads are opaque.
pub fn panic_payload_text(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "Box<dyn Any>".into())
}

/// Whether the current panic will be reported and contained by the retained callback owner.
pub fn retained_callback_active() -> bool {
    RETAINED_CALLBACK.try_with(Cell::get).unwrap_or(false)
}

// Interrupt wrapper code without invoking the panic hook a second time.
fn unwind_through_wrapper() -> ! {
    struct DelegatedEnginePanic;
    std::panic::resume_unwind(Box::new(DelegatedEnginePanic))
}

fn engine_panic_pending() -> bool {
    DELEGATED_ENGINE_PANIC.with(|panic| panic.borrow().is_some())
}

fn drop_contained<T>(value: T) {
    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(value))) {
        // A panic payload can itself have a panicking destructor.
        std::mem::forget(payload);
    }
}

#[derive(PartialEq, Eq)]
enum Invocation {
    Isolated,
    Dispose,
}

enum CallFailure {
    Disabled(&'static str),
    Panicked(&'static str, Option<String>),
}

impl CallFailure {
    fn into_message(self) -> String {
        match self {
            Self::Disabled(kind) => format!("{kind} owner is disabled"),
            Self::Panicked(kind, None) => format!("{kind} callback panicked; it is disabled"),
            Self::Panicked(kind, Some(message)) => {
                format!("{kind} callback panicked; it is disabled: {message}")
            }
        }
    }
}

struct RetainedCallbackScope(bool);
impl Drop for RetainedCallbackScope {
    fn drop(&mut self) {
        RETAINED_CALLBACK.set(self.0);
    }
}

/// Owner identity and poison state for one retained mod callback.
// no wire: the callback's enablement and poison state belong to this process.
// exhaustive: engine-only callback bookkeeping; private fields prevent downstream construction.
#[derive(Clone, Debug)]
pub struct CallbackOwner {
    id: &'static str,
    binding: OwnerBinding,
    poisoned: Arc<AtomicBool>,
}

// Retained work can run on a worker, outside the thread which issued its owner.
#[derive(Clone, Copy)]
struct OwnerBinding {
    index: Option<usize>,
    installed: Option<&'static crate::runtime::Installed>,
}

impl std::fmt::Debug for OwnerBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnerBinding")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

impl OwnerBinding {
    fn current(index: Option<usize>) -> Self {
        Self {
            index,
            installed: crate::runtime::installed(),
        }
    }

    fn installed(self) -> Option<&'static crate::runtime::Installed> {
        self.installed.or_else(crate::runtime::installed)
    }

    fn enabled(self, poisoned: &AtomicBool) -> bool {
        !poisoned.load(Ordering::Acquire)
            && self
                .index
                .zip(self.installed())
                .is_none_or(|(index, installed)| !installed.disabled[index].load(Ordering::Relaxed))
    }
}

impl CallbackOwner {
    /// Bind a dispatch-attributed registration to its installed mod slot once, at freeze time.
    pub fn new(id: &'static str) -> Option<Self> {
        let index = crate::__private::mod_index(id)?;
        Some(Self {
            id,
            binding: OwnerBinding::current(Some(index)),
            poisoned: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(crate) fn for_index(id: &'static str, index: usize) -> Self {
        Self {
            id,
            binding: OwnerBinding {
                index: Some(index),
                installed: None,
            },
            poisoned: Arc::new(AtomicBool::new(false)),
        }
    }

    fn scheduled(id: &str) -> Option<&'static Self> {
        let installed = crate::runtime::installed()?;
        let &index = installed.by_id.get(id)?;
        Some(&installed.scheduled_tasks[index])
    }

    /// Run a delayed task using its installed owner's shared state. A panic returns a diagnostic
    /// for the server to relay after rolling back the task's transaction.
    pub fn call_scheduled_task<H: crate::ServerHost>(
        id: &'static str,
        host: &mut H,
        invoke: impl FnOnce(&mut H),
    ) -> Result<(), String> {
        let state = Self::scheduled(id).map(|owner| (owner.binding.index, owner.poisoned.as_ref()));
        match run_scheduled(id, state, "schedule_task", || {
            crate::run_transaction(host, |host, _| {
                invoke(host);
                Ok::<(), ()>(())
            })
            .ok();
        }) {
            Err(ScheduledFailure::Panicked(diagnostic)) => Err(diagnostic),
            _ => Ok(()),
        }
    }

    /// Drop a withdrawn task, using the same session poison state as due tasks. An attributed
    /// task without an installed owner still needs containment, but has no state to disable.
    pub fn dispose_scheduled_task<T>(id: &'static str, task: T) {
        let uninstalled = AtomicBool::new(false);
        let (index, poisoned) = Self::scheduled(id).map_or((None, &uninstalled), |owner| {
            (owner.binding.index, owner.poisoned.as_ref())
        });
        let _ = call_opt(
            id,
            OwnerBinding::current(index),
            poisoned,
            "scheduled task",
            "schedule_task_drop",
            Invocation::Dispose,
            || drop(task),
        );
    }

    /// Bind a callback owner by mod id, resolving its installed slot when present.
    pub fn for_mod(id: &'static str) -> Self {
        Self {
            id,
            binding: OwnerBinding::current(crate::__private::mod_index(id)),
            poisoned: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The owning mod's identifier.
    pub fn id(&self) -> &'static str {
        self.id
    }

    /// Whether this callback is still enabled for this session.
    pub fn is_enabled(&self) -> bool {
        self.binding.enabled(&self.poisoned)
    }

    /// Invoke retained code with its owner's enablement, attribution and panic policy.
    pub fn call<T>(
        &self,
        kind: &'static str,
        operation: &'static str,
        invoke: impl FnOnce() -> T,
    ) -> Result<T, String> {
        self.call_isolated(kind, operation, invoke)
            .map_err(|why| format!("mod {}: {}", self.id, why.into_message()))
    }

    /// Invoke a callback on live state, restoring the state only if this call panics.
    /// A normal answer (including a veto) keeps the current state.
    pub fn call_with_panic_rollback<T, S: Clone>(
        &self,
        kind: &'static str,
        operation: &'static str,
        state: &mut S,
        invoke: impl FnOnce(&mut S) -> T,
    ) -> Result<T, String> {
        let mut before = None;
        let result = self.call_isolated(kind, operation, || {
            before = Some(state.clone());
            invoke(state)
        });
        if matches!(&result, Err(CallFailure::Panicked(..)))
            && let Some(before) = before
        {
            *state = before;
        }
        result.map_err(|why| format!("mod {}: {}", self.id, why.into_message()))
    }

    fn call_isolated<T>(
        &self,
        kind: &'static str,
        operation: &'static str,
        invoke: impl FnOnce() -> T,
    ) -> Result<T, CallFailure> {
        call_opt(
            self.id,
            self.binding,
            &self.poisoned,
            kind,
            operation,
            Invocation::Isolated,
            invoke,
        )
    }

    /// Dispose of an owned mod value under its panic policy, even if ordinary calls are disabled.
    pub fn dispose<T>(&self, kind: &'static str, operation: &'static str, value: T) {
        let _ = call_opt(
            self.id,
            self.binding,
            &self.poisoned,
            kind,
            operation,
            Invocation::Dispose,
            || drop(value),
        );
    }

    /// Execute the engine body delegated by a retained wrapper, outside all mod scopes.
    pub fn call_engine<T>(invoke: impl FnOnce() -> T) -> T {
        if !retained_callback_active() {
            return invoke();
        }
        if engine_panic_pending() {
            unwind_through_wrapper();
        }
        let _contained = RetainedCallbackScope(RETAINED_CALLBACK.replace(false));
        let _suspended = SuspendedPanicScope::enter();
        let _guard = crate::dispatch_scope::ModGuard::unattributed();
        let result =
            crate::runtime::without_callback_scope(|| catch_unwind(AssertUnwindSafe(invoke)));
        result.unwrap_or_else(|payload| {
            DELEGATED_ENGINE_PANIC.set(Some(payload));
            unwind_through_wrapper();
        })
    }
}

/// An engine-retained value whose destructor runs under its mod callback's owner.
// no wire: retained callback values and their ownership are process-local.
// exhaustive: engine-only wrapper with private state.
pub struct CallbackValue<T> {
    owner: CallbackOwner,
    kind: &'static str,
    drop_operation: &'static str,
    value: Option<T>,
}

impl<T> CallbackValue<T> {
    /// Retain `value` and dispose of it under `owner` when this wrapper drops.
    pub fn new(
        owner: CallbackOwner,
        kind: &'static str,
        drop_operation: &'static str,
        value: T,
    ) -> Self {
        Self {
            owner,
            kind,
            drop_operation,
            value: Some(value),
        }
    }

    /// The owner to use when invoking the retained callback.
    pub fn owner(&self) -> &CallbackOwner {
        &self.owner
    }

    /// Borrow the value, which is present until this wrapper is dropped.
    pub fn value(&self) -> &T {
        self.value.as_ref().expect("live callback value")
    }
}

impl<T> Drop for CallbackValue<T> {
    fn drop(&mut self) {
        if let Some(value) = self.value.take() {
            self.owner.dispose(self.kind, self.drop_operation, value);
        }
    }
}

pub(crate) fn enabled(index: usize, poisoned: &AtomicBool) -> bool {
    enabled_opt(Some(index), poisoned)
}

fn enabled_opt(index: Option<usize>, poisoned: &AtomicBool) -> bool {
    index.is_none_or(crate::mod_index_on) && !poisoned.load(Ordering::Acquire)
}

pub(crate) fn call<T>(
    owner: &'static str,
    index: usize,
    poisoned: &AtomicBool,
    kind: &'static str,
    operation: &'static str,
    invoke: impl FnOnce() -> T,
) -> Result<T, String> {
    call_opt(
        owner,
        OwnerBinding::current(Some(index)),
        poisoned,
        kind,
        operation,
        Invocation::Isolated,
        invoke,
    )
    .map_err(CallFailure::into_message)
}

/// Ask an optional provider under its owner. Faults are reported at the callback boundary;
/// the lookup only needs to know whether it answered.
pub(crate) fn call_for_lookup<T>(
    owner: &'static str,
    index: usize,
    poisoned: &AtomicBool,
    kind: &'static str,
    operation: &'static str,
    invoke: impl FnOnce() -> T,
) -> Option<T> {
    call_opt(
        owner,
        OwnerBinding::current(Some(index)),
        poisoned,
        kind,
        operation,
        Invocation::Isolated,
        invoke,
    )
    .ok()
}

/// Failure of a retained callback, distinct from the result returned by the mod itself.
// no wire: an in-process dispatch outcome; never sent to the peer.
// exhaustive: engine-only dispatch result.
#[derive(Debug)]
pub enum CallbackFailure {
    /// A refusal, not a panic.
    Refused(String),
    /// The callback just panicked and the shared boundary reported it.
    Panicked,
    /// A previous panic disabled the callback; it did not run.
    Poisoned,
}

pub(crate) fn call_typed<T>(
    owner: &'static str,
    index: usize,
    poisoned: &AtomicBool,
    kind: &'static str,
    operation: &'static str,
    invoke: impl FnOnce() -> T,
) -> Result<T, CallbackFailure> {
    if poisoned.load(Ordering::Acquire) {
        let _guard = crate::mark_mod(owner, operation);
        let _ = contain_unwind(|| drop(invoke));
        return Err(CallbackFailure::Poisoned);
    }
    if !crate::mod_index_on(index) {
        let _guard = crate::mark_mod(owner, operation);
        let _ = contain_unwind(|| drop(invoke));
        return Err(CallbackFailure::Refused(format!(
            "{kind} owner is disabled"
        )));
    }
    call_opt(
        owner,
        OwnerBinding::current(Some(index)),
        poisoned,
        kind,
        operation,
        Invocation::Isolated,
        invoke,
    )
    .map_err(|failure| match failure {
        CallFailure::Disabled(_) => CallbackFailure::Refused(failure.into_message()),
        CallFailure::Panicked(..) => CallbackFailure::Panicked,
    })
}

enum ScheduledFailure {
    Disabled,
    Panicked(String),
}

fn run_scheduled<T>(
    owner: &'static str,
    state: Option<(Option<usize>, &AtomicBool)>,
    operation: &'static str,
    invoke: impl FnOnce() -> T,
) -> Result<T, ScheduledFailure> {
    let _guard = crate::mark_mod(owner, operation);
    match contain_unwind(|| {
        if state.is_some_and(|(index, poisoned)| enabled_opt(index, poisoned)) {
            Some(invoke())
        } else {
            // Captures also belong to the owner if the callable is discarded without running.
            drop(invoke);
            None
        }
    }) {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Err(ScheduledFailure::Disabled),
        Err(diagnostic) => {
            if let Some((_, poisoned)) = state {
                poisoned.store(true, Ordering::Release);
            }
            Err(ScheduledFailure::Panicked(diagnostic))
        }
    }
}

fn call_opt<T>(
    owner: &'static str,
    binding: OwnerBinding,
    poisoned: &AtomicBool,
    kind: &'static str,
    operation: &'static str,
    invocation: Invocation,
    invoke: impl FnOnce() -> T,
) -> Result<T, CallFailure> {
    if invocation != Invocation::Dispose && !binding.enabled(poisoned) {
        let _guard = crate::mark_mod(owner, operation);
        let _ = contain_unwind(|| drop(invoke));
        return Err(CallFailure::Disabled(kind));
    }
    let _guard = crate::mark_mod(owner, operation);
    let contained = RetainedCallbackScope(RETAINED_CALLBACK.replace(true));
    let scope = PanicScope::enter();
    let mut diagnostic = None;
    let result = crate::runtime::contain_callback(invoke, |payload| {
        if engine_panic_pending() {
            return;
        }
        let first = !poisoned.swap(true, Ordering::AcqRel);
        let msg = scope
            .take_message()
            .unwrap_or_else(|| panic_payload_text(payload));
        if first || invocation == Invocation::Dispose {
            crate::runtime::log_and_queue_named_panic(
                binding.installed(),
                operation,
                owner,
                None,
                payload,
            );
            diagnostic = Some(msg);
        }
    });
    if engine_panic_pending() {
        // Ignore the wrapper's outcome, even if it caught the unwind and returned normally.
        drop_contained(result);
        drop(scope);
        if contained.0 {
            unwind_through_wrapper();
        }
        drop(contained);
        let payload = DELEGATED_ENGINE_PANIC.take().expect("pending engine panic");
        std::panic::resume_unwind(payload);
    }
    result.ok_or(CallFailure::Panicked(kind, diagnostic))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn a_panicking_payload_destructor_preserves_the_original_callback_diagnostic() {
        use std::sync::{Arc, Mutex};

        // The process-wide hook must not replace the hook of concurrent tests.
        const CHILD: &str = "SUNRISE_CALLBACK_PANIC_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "callback_owner::tests::a_panicking_payload_destructor_preserves_the_original_callback_diagnostic",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("test result: ok. 1 passed;"),
                "the isolated panic-hook test did not pass exactly one test:\n{stdout}{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        const ORIGINAL: &str = "original opaque callback panic";
        const DESTRUCTOR: &str = "panic in the payload destructor";
        struct PanicOnDrop(Arc<AtomicBool>);
        impl Drop for PanicOnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Relaxed);
                std::panic::panic_any(DESTRUCTOR);
            }
        }

        let recorded = Arc::new(Mutex::new(Vec::new()));
        let seen = recorded.clone();
        std::panic::set_hook(Box::new(move |info| {
            let diagnostic = if info.payload().is::<PanicOnDrop>() {
                ORIGINAL
            } else if info.payload().downcast_ref::<&str>().copied() == Some(DESTRUCTOR) {
                DESTRUCTOR
            } else {
                return;
            };
            let contained = record_contained_panic(&|| diagnostic.to_owned());
            seen.lock().unwrap().push((diagnostic, contained));
        }));

        let dropped = Arc::new(AtomicBool::new(false));
        let poisoned = AtomicBool::new(false);
        let result: Result<(), _> = call("owner", 0, &poisoned, "test", "test", || {
            std::panic::panic_any(PanicOnDrop(dropped.clone()));
        });
        drop(std::panic::take_hook());
        assert!(dropped.load(Ordering::Relaxed));
        assert_eq!(
            recorded.lock().unwrap().as_slice(),
            [(ORIGINAL, true), (DESTRUCTOR, true)]
        );
        assert_eq!(
            result.unwrap_err(),
            "test callback panicked; it is disabled: original opaque callback panic"
        );
        assert!(poisoned.load(Ordering::Acquire));
    }
}
