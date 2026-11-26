//! Named ticket controllers (NeoForge's `TicketController` and `RegisterTicketControllersEvent`): a
//! mod registers each controller once at startup, from [`Hooks::register`](crate::Hooks::register),
//! and holds the [`TicketController`] it gets back to hold chunks with.

use crate::{
    LevelRef, ServerHost, TicketValidation,
    runtime::RetainedCallback,
    world::mod_chunk_tickets::{ControllerTickets, ModTicket},
};
use std::{
    collections::BTreeMap,
    hash::BuildHasher,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Issue a non-zero opaque token unique to one registration entry.
/// Mixing a monotonic counter with a per-process salt through SplitMix64's bijection keeps
/// tokens non-sequential and prevents handles from one registration table matching another.
pub(crate) fn issue_token() -> u64 {
    static SALT: OnceLock<u64> = OnceLock::new();
    let salt = *SALT.get_or_init(|| {
        std::collections::hash_map::RandomState::new().hash_one(&SALT as *const _ as usize)
    });
    loop {
        let seq = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
        let mut x = seq.wrapping_add(salt).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        let token = x ^ (x >> 31);
        if token != 0 {
            return token;
        }
    }
}

// no wire: retained executable code and a borrow of staged tickets stay in-process.
/// Validate one controller's saved chunk tickets in the supplied server level
/// (NeoForge's `LoadingValidationCallback.validateTickets`). The callback runs
/// after `level_load` and before the first tick, only when that controller has
/// saved rows in the level. At startup all built-in levels are available before
/// validation begins. Remove stale tickets through the borrowed validation view;
/// the survivors force their chunks after validation. A panic is reported and
/// disables only this validator for the session and discards its removals.
pub type LoadingValidationCallback =
    dyn Fn(&mut dyn ServerHost, LevelRef, &mut dyn TicketValidation) + Send + Sync + 'static;

// no wire: retained code, attribution and session poison state stay in-process.
/// One frozen controller's callback and its registered owner.
// hidden: the authoring bridge moves the binding without inspecting it.
#[doc(hidden)]
#[derive(Clone)]
// exhaustive: private fields keep callback ownership inside the binding.
pub struct RegisteredTicketValidator {
    /// Taken only by `Drop`, which retires it inside the owner's containment.
    callback: Option<Arc<LoadingValidationCallback>>,
    owner: RetainedCallback,
}

impl Drop for RegisteredTicketValidator {
    fn drop(&mut self) {
        if let Some(callback) = self.callback.take() {
            self.owner.dispose("TicketValidator::drop", callback);
        }
    }
}

impl RegisteredTicketValidator {
    pub(crate) fn new(callback: Arc<LoadingValidationCallback>, owner: RetainedCallback) -> Self {
        Self {
            callback: Some(callback),
            owner,
        }
    }

    fn validate(&self, host: &mut dyn ServerHost, level: LevelRef, rows: &mut Vec<ModTicket>) {
        let Some(callback) = &self.callback else {
            return;
        };
        let mut validation = ControllerTickets::new(rows);
        if self
            .owner
            .call(|| callback(host, level, &mut validation))
            .is_ok()
        {
            validation.publish_removals();
        }
    }
}

/// What the game knows about a registered controller.
pub(crate) struct ControllerInfo {
    /// The namespaced id, `mymod:quarry`, which the tickets are saved under.
    pub(crate) id: Box<str>,
    /// Opaque registration identity matched against issued [`TicketController`] handles.
    pub(crate) token: u64,
    /// The controller's retained saved-ticket validator, if registered.
    pub(crate) validator: Option<RegisteredTicketValidator>,
}

/// A ticket controller a mod registered with
/// [`Registrar::register_ticket_controller`](crate::TicketsRegistrar::register_ticket_controller): the
/// handle `chunk.set_mod_forced` and `level.visit_mod_forced_chunks` take. Only the registrar makes
/// one, bound to an opaque registration identity, and it is `Copy`: keep it from `register`, for
/// example in a `OnceLock`. Active chunk-ticket reads and writes authorize by this registry-issued
/// handle rather than the current caller; separate controllers cannot read or release each other's
/// tickets, a name or slot alone cannot forge a handle, and a handle from a refused registration
/// or another registration table does nothing. Use its namespaced `id()` when you need to store a
/// controller name.
// no wire: the handle resolves only in the registry that issued it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct TicketController {
    /// Its place in the installed controller table.
    pub(crate) slot: u32,
    /// Opaque registration identity issued alongside the table entry; 0 on refused handles.
    pub(crate) token: u64,
}

impl TicketController {
    /// The controller's namespaced id (`mymod:quarry`), as its tickets are saved under it; empty
    /// for a registration that was refused or a handle from another registration table.
    pub fn id(self) -> &'static str {
        self.info().map_or("", |info| &info.id)
    }

    pub(crate) fn active_slot(self) -> Option<u32> {
        self.info().map(|_| self.slot)
    }

    fn info(self) -> Option<&'static ControllerInfo> {
        if self.token == 0 {
            return None;
        }
        controllers()
            .get(self.slot as usize)
            .filter(|info| info.token == self.token)
    }
}

/// Save-serialization lookup: map a stored controller slot to its namespaced id without issuing a
/// [`TicketController`] handle.
pub(crate) fn id_for_slot(slot: u32) -> Option<&'static str> {
    controllers().get(slot as usize).map(|info| &*info.id)
}

/// Save-deserialization lookup: map a saved controller id to its installed slot without issuing a
/// [`TicketController`] handle.
pub(crate) fn slot_for_id(id: &str) -> Option<u32> {
    controllers()
        .iter()
        .position(|info| &*info.id == id)
        .map(|slot| slot as u32)
}

// no wire: a process-local authority owned by the engine's level-open path.
// exhaustive: downstream callers cannot construct this authority.
// hidden: engine-only startup ticket activation authority exposed through __private::game.
/// The engine's authority to activate a level's saved mod tickets.
#[doc(hidden)]
pub struct TicketActivation(ActivationTable);

enum ActivationTable {
    Installed,
    #[cfg(any(test, feature = "testing"))]
    Fixture(&'static [ControllerInfo]),
}

impl TicketActivation {
    /// Hand the sole installed-table authority to engine startup, before loading any native code.
    /// There is no way to recover it after that handoff.
    pub fn take_for_startup() -> Option<Self> {
        static CLAIMED: OnceLock<()> = OnceLock::new();
        Self::claim(&CLAIMED)
    }

    fn claim(slot: &OnceLock<()>) -> Option<Self> {
        slot.set(()).ok().map(|()| Self(ActivationTable::Installed))
    }

    /// Obtain authority over only the controller table installed by the current test fixture.
    #[cfg(any(test, feature = "testing"))]
    pub fn for_test_scope() -> Option<Self> {
        TEST_CONTROLLERS
            .get()
            .map(|rows| Self(ActivationTable::Fixture(rows)))
    }

    /// Validate the level's staged saved rows, returning only those ready to activate.
    pub fn validate(
        &self,
        host: &mut dyn ServerHost,
        level: LevelRef,
        loaded: Vec<ModTicket>,
    ) -> Vec<ModTicket> {
        let table = match self.0 {
            ActivationTable::Installed => crate::parts::installed_controllers(),
            #[cfg(any(test, feature = "testing"))]
            ActivationTable::Fixture(rows) => rows,
        };
        validate_saved_tickets(table, host, level, loaded)
    }
}

/// Run each registered controller's bound saved-ticket validator in slot order.
fn validate_saved_tickets(
    table: &[ControllerInfo],
    host: &mut dyn ServerHost,
    level: LevelRef,
    loaded: Vec<ModTicket>,
) -> Vec<ModTicket> {
    let mut ready = Vec::new();
    let mut groups = BTreeMap::<u32, (&RegisteredTicketValidator, Vec<ModTicket>)>::new();
    for ticket in loaded {
        if let Some(validator) = table
            .get(ticket.controller as usize)
            .and_then(|info| info.validator.as_ref())
            .filter(|validator| validator.owner.enabled())
        {
            groups
                .entry(ticket.controller)
                .or_insert_with(|| (validator, Vec::new()))
                .1
                .push(ticket);
        } else {
            ready.push(ticket);
        }
    }
    for (validator, mut rows) in groups.into_values() {
        validator.validate(host, level, &mut rows);
        ready.append(&mut rows);
    }
    ready
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    /// The controllers a test's mods registered (`testing::with_dispatch`), in place of the
    /// process-wide table.
    pub(crate) static TEST_CONTROLLERS: std::cell::Cell<Option<&'static [ControllerInfo]>> =
        const { std::cell::Cell::new(None) };
}

fn controllers() -> &'static [ControllerInfo] {
    #[cfg(any(test, feature = "testing"))]
    if let Some(table) = TEST_CONTROLLERS.get() {
        return table;
    }
    crate::parts::installed_controllers()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        __private::parts_installation::register_ticket_controller,
        Hooks, ModEntry,
        testing::{FakeHost, with_controller_tables, with_dispatch},
        world::mod_chunk_tickets::TicketOwner,
    };
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn only_the_first_claim_receives_installed_ticket_activation_authority() {
        let slot = OnceLock::new();
        let engine = TicketActivation::claim(&slot).unwrap();
        assert!(matches!(engine.0, ActivationTable::Installed));
        assert!(TicketActivation::claim(&slot).is_none());
        assert!(TicketActivation::for_test_scope().is_none());
    }

    #[test]
    fn a_panicking_ticket_validator_leaves_loaded_tickets_and_other_controllers_untouched() {
        struct Mod;
        #[mod_api::__subscriptions]
        impl Hooks for Mod {}

        const POS: [i32; 3] = [1, 64, 1];
        let calls = Arc::new(AtomicUsize::new(0));
        let (failing, _) = register_ticket_controller(
            0,
            "failing:controller".into(),
            "failing",
            0,
            Some(Arc::new({
                let calls = calls.clone();
                move |_: &mut dyn ServerHost, _, validation: &mut dyn TicketValidation| {
                    if validation.remove_block_ticket(POS, [2, 3], false) {
                        calls.fetch_add(1, Ordering::Relaxed);
                    }
                    panic!("failed after selecting a ticket");
                }
            })),
        );
        let (healthy, _) = register_ticket_controller(
            1,
            "healthy:controller".into(),
            "healthy",
            1,
            Some(Arc::new(
                |_: &mut dyn ServerHost, _, validation: &mut dyn TicketValidation| {
                    validation.remove_block_ticket(POS, [2, 3], false);
                },
            )),
        );
        let ticket = |controller| ModTicket {
            controller,
            owner: TicketOwner::Block(POS),
            chunk: (2, 3),
            natural_spawning: false,
        };
        with_dispatch(
            vec![
                ModEntry::new("failing", Box::new(Mod)),
                ModEntry::new("healthy", Box::new(Mod)),
            ]
            .into(),
            || {
                with_controller_tables(vec![failing, healthy], vec![], || {
                    let mut host = FakeHost::new();
                    let level = host.level.level();
                    let activation = TicketActivation::for_test_scope().unwrap();
                    for _ in 0..2 {
                        assert_eq!(
                            activation.validate(&mut host, level, vec![ticket(0), ticket(1)]),
                            [ticket(0)],
                        );
                    }
                    assert_eq!(calls.load(Ordering::Relaxed), 1);
                    assert_eq!(
                        crate::__private::take_hook_panics(),
                        [crate::__private::HookPanicReport {
                            mod_id: "failing",
                            hook: "validate_tickets",
                        }],
                    );
                });
            },
        );
    }
}
