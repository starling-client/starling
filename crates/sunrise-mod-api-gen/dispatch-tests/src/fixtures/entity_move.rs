//! The global cancel dispatcher keeps accepted outputs across listener failures.

/// What the fixture listener heard: each requested delta and cancel flag.
type Heard = Vec<([f64; 3], bool)>;

struct Mover {
    answer: Answer,
    heard: Arc<Mutex<Heard>>,
}

#[derive(Clone, Copy)]
enum Answer {
    Replace([f64; 3]),
    Panic,
    Invalid([f64; 3]),
    Cancel,
    Continue,
}

#[mod_api::__subscriptions]
impl Hooks for Mover {
    fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
        self.heard
            .lock()
            .unwrap()
            .push((*event.delta, event.cancelled));
        match self.answer {
            Answer::Replace(delta) => *event.delta = delta,
            Answer::Panic => {
                *event.delta = [9.0; 3];
                event.cancelled = true;
                panic!("the fixture's move listener unwinds")
            }
            Answer::Invalid(delta) => {
                *event.delta = delta;
                event.cancelled = true;
                return Flow::Cancel;
            }
            Answer::Cancel => return Flow::Cancel,
            Answer::Continue => {}
        }
        Flow::Continue
    }
}

fn mover(id: &'static str, answer: Answer) -> (Arc<Mutex<Heard>>, ModEntry) {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let entry = ModEntry::new(
        id,
        Box::new(Mover {
            answer,
            heard: Arc::clone(&heard),
        }),
    );
    (heard, entry)
}

fn dispatch(mut delta: [f64; 3]) -> ([f64; 3], bool) {
    let mut canceled = false;
    mod_api::dispatch_entity_move(
        &Loop("server"),
        7,
        "minecraft:zombie",
        MoveCause::SelfMovement,
        &mut delta,
        &mut canceled,
    );
    (delta, canceled)
}

#[test]
fn a_panicking_move_listener_loses_its_proposal_but_the_walk_continues() {
    let accepted = [3.0, 4.0, 5.0];
    let (first_heard, first) = mover("accepted", Answer::Replace(accepted));
    let (panic_heard, panicker) = mover("panicker", Answer::Panic);
    let (later_heard, later) = mover("later", Answer::Continue);
    install(vec![first, panicker, later].into());

    assert_eq!(dispatch([1.0, 0.0, 0.0]), (accepted, false));
    assert_eq!(*later_heard.lock().unwrap(), [(accepted, false)]);
    assert_eq!(dispatch([2.0, 0.0, 0.0]), (accepted, false));
    assert_eq!(
        *first_heard.lock().unwrap(),
        [([1.0, 0.0, 0.0], false), ([2.0, 0.0, 0.0], false)]
    );
    assert_eq!(*panic_heard.lock().unwrap(), [(accepted, false)]);
    assert_eq!(*later_heard.lock().unwrap(), [(accepted, false); 2]);
    assert!(runtime::poisoned(HookId::EntityMove, "panicker"));
    assert!(!runtime::poisoned(HookId::EntityMove, "accepted"));
    assert!(!runtime::poisoned(HookId::EntityMove, "later"));
}

#[test]
fn a_failed_receiver_cannot_clear_an_accepted_move_veto() {
    struct ClearThenPanic(Arc<Mutex<Heard>>);
    #[mod_api::__subscriptions]
    impl Hooks for ClearThenPanic {
        #[listener(receive_cancelled)]
        fn entity_move(&self, _: &dyn LoopContext, event: &mut EntityMoveEvent<'_>) -> Flow {
            self.0.lock().unwrap().push((*event.delta, event.cancelled));
            event.cancelled = false;
            *event.delta = [9.0; 3];
            panic!("the veto stands")
        }
    }

    let (_, veto) = mover("veto", Answer::Cancel);
    let heard = Arc::new(Mutex::new(Vec::new()));
    let failed = ModEntry::new("failed_receiver", Box::new(ClearThenPanic(heard.clone())));
    let (skipped_heard, skipped) = mover("skipped", Answer::Continue);
    install(vec![veto, failed, skipped].into());
    assert_eq!(dispatch([1.0; 3]), ([1.0; 3], true));
    assert_eq!(*heard.lock().unwrap(), [([1.0; 3], true)]);
    assert!(runtime::poisoned(HookId::EntityMove, "failed_receiver"));
    assert!(skipped_heard.lock().unwrap().is_empty());
}

#[test]
fn an_invalid_move_discards_its_delta_and_veto_and_reports_each_invocation() {
    let delta = [65.0, -66.0, 67.0];
    for proposed in [[f64::NAN, 0.0, 0.0], [0.0, 0.0, f64::INFINITY]] {
        let (_, first) = mover("accepted", Answer::Replace(delta));
        let (invalid_heard, invalid) = mover("invalid", Answer::Invalid(proposed));
        let (later_heard, later) = mover("later", Answer::Continue);
        install(vec![first, invalid, later].into());

        let refusals = capture_output_refusals(|| {
            assert_eq!(dispatch([1.0; 3]), (delta, false));
            assert_eq!(dispatch([2.0; 3]), (delta, false));
        });
        assert_eq!(refusals, [("invalid", "entity_move", "delta"); 2]);
        assert_eq!(*invalid_heard.lock().unwrap(), [(delta, false); 2]);
        assert_eq!(*later_heard.lock().unwrap(), [(delta, false); 2]);
        assert!(!runtime::poisoned(HookId::EntityMove, "invalid"));
    }
}

mod root {
    mod runtime {
        pub(super) enum Callback {
            Hook(super::HookId),
            IndependentHook(super::HookId),
        }

        impl From<super::HookId> for Callback {
            fn from(hook: super::HookId) -> Self {
                Self::Hook(hook)
            }
        }

        pub(super) fn invoke<T>(
            callback: impl Into<Callback>,
            _: &'static str,
            mod_id: &'static str,
            call: impl FnOnce() -> T,
        ) -> Option<T> {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)) {
                Ok(value) => Some(value),
                Err(payload) => {
                    let (Callback::Hook(hook) | Callback::IndependentHook(hook)) = callback.into();
                    poison(hook, mod_id);
                    // The real boundary also contains the destructor of a panic payload.
                    if let Err(secondary) =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(payload)))
                    {
                        std::mem::forget(secondary);
                    }
                    None
                }
            }
        }

        pub(super) fn invoke_without_disabling<T>(
            _: &'static str,
            _: &'static str,
            call: impl FnOnce() -> T,
        ) -> Option<T> {
            Some(call())
        }

        pub(super) fn report_output_refusal(
            owner: &'static str,
            hook: &'static str,
            field: &'static str,
        ) {
            super::OUTPUT_REFUSALS.with_borrow_mut(|refusals| {
                if let Some(refusals) = refusals {
                    refusals.push((owner, hook, field));
                }
            });
        }

        pub(super) fn discard_registration<T>(
            (hook, name): (super::HookId, &'static str),
            owner: Option<&'static str>,
            value: T,
        ) {
            invoke(hook, name, owner.unwrap_or("unknown"), || drop(value));
        }
        pub(super) fn run_isolated_part_output<
            P: ?Sized,
            H: super::ServerHost + ?Sized,
            O: Clone,
        >(
            parts: &[&'static P],
            host: &mut H,
            output: &mut O,
            _: &'static str,
            _: &'static str,
            mut invoke: impl FnMut(&P, &mut H, &mut O),
        ) {
            for &part in parts {
                let mut candidate = output.clone();
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    invoke(part, host, &mut candidate);
                }))
                .is_ok()
                {
                    *output = candidate;
                }
            }
        }

        pub struct ConsumeEffectBinding(&'static dyn super::Hooks);
        impl ConsumeEffectBinding {
            pub(super) fn invoke(self, call: impl FnOnce(&dyn super::Hooks)) -> bool {
                call(self.0);
                true
            }
        }

        pub(super) fn entered_with_id_matching<'a>(
            hook_id: super::HookId,
            hook: &'static str,
            allow: impl FnMut(&str) -> bool + 'a,
        ) -> impl Iterator<Item = (&'static str, &'static dyn super::Hooks)> + 'a {
            listening_matching(hook_id, allow)
                .map(move |(_, entry)| (entry.id, super::enter_listener(entry, hook).1))
        }

        pub(super) fn listening_matching<'a>(
            hook: super::HookId,
            mut allow: impl FnMut(&str) -> bool + 'a,
        ) -> impl Iterator<Item = (bool, &'static super::ModEntry)> + 'a {
            super::listening(hook).filter(move |(_, entry)| allow(entry.id))
        }

        thread_local! {
            static ACTIVE: std::cell::RefCell<Vec<(usize, &'static str)>> = const {
                std::cell::RefCell::new(Vec::new())
            };
        }

        pub(super) fn listener_is_inactive(hook: super::HookId, _: &str, id: &str) -> bool {
            ACTIVE.with_borrow(|active| !active.contains(&(hook as usize, id)))
        }

        pub(super) struct ActiveListener;

        impl ActiveListener {
            pub(super) fn enter(hook: super::HookId, id: &'static str) -> Self {
                ACTIVE.with_borrow_mut(|active| active.push((hook as usize, id)));
                Self
            }
        }

        impl Drop for ActiveListener {
            fn drop(&mut self) {
                ACTIVE.with_borrow_mut(|active| active.pop());
            }
        }

        pub(super) struct ListenerScope;
        impl ListenerScope {
            pub(super) fn new() -> Self {
                Self
            }
            pub(super) fn enter(
                &mut self,
                entry: &'static super::ModEntry,
                hook: &'static str,
            ) -> &'static dyn super::Hooks {
                super::enter_listener(entry, hook).1
            }
        }
        thread_local! {
            /// The listeners dispatch poisoned, as (hook ordinal, id) pairs — the stand-in
            /// for the real runtime's per-hook sets.
            pub(super) static POISONED: std::cell::RefCell<std::collections::HashSet<(usize, &'static str)>> =
                std::cell::RefCell::new(std::collections::HashSet::new());
        }
        pub(super) fn poison(hook: super::HookId, id: &'static str) {
            POISONED.with(|poisoned| poisoned.borrow_mut().insert((hook as usize, id)));
        }
        pub(super) fn poisoned(hook: super::HookId, id: &str) -> bool {
            POISONED.with(|poisoned| {
                poisoned
                    .borrow()
                    .iter()
                    .any(|(poisoned_hook, poisoned_id)| {
                        *poisoned_hook == hook as usize && *poisoned_id == id
                    })
            })
        }
    }
}
