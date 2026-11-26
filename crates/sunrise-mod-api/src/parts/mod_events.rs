//! Validation of mod event listeners: their owner, their event's id, and one type per id.
use super::*;

impl ModEventsRegistrar for PartRegistrar {
    fn add_mod_event_listener(
        &mut self,
        subscription: ModEventSubscription,
    ) -> Result<(), crate::DeclarationError> {
        use crate::DeclarationError;
        let ModEventSubscription {
            id,
            event,
            type_name,
            client,
            priority,
            receive_cancelled,
            listener,
            ..
        } = subscription;
        let result = (|| {
            if mod_vocab::__private::parts_installation::parts_installed() {
                return Err(DeclarationError::RegistrationClosed { id: id.into() });
            }
            if crate::Identifier::parse_namespaced(id).is_none() || id.starts_with("minecraft:") {
                return Err(DeclarationError::InvalidId { id: id.into() });
            }
            if self.mod_type != Some(listener.mod_type) {
                return Err(DeclarationError::Rejected {
                    id: id.into(),
                    reason: "the listener is not a method of this mod's Hooks type".into(),
                });
            }
            let clash = self
                .parts
                .mod_events
                .iter()
                .filter(|(other, _)| **other != event)
                .flat_map(|(_, listeners)| listeners)
                .find(|other| other.event_id == id);
            if let Some(other) = clash {
                return Err(DeclarationError::Rejected {
                    id: id.into(),
                    reason: format!(
                        "event {id} is listened to with two types, {} by {} and \
                         {type_name} by {}",
                        other.event_type,
                        other.owner.id(),
                        self.mod_id
                    ),
                });
            }
            let listeners = self.parts.mod_events.entry(event).or_default();
            let index = listeners.partition_point(|other| other.priority >= priority);
            listeners.insert(
                index,
                ModEventListener {
                    owner: mod_vocab::__private::game::CallbackOwner::for_index(
                        self.mod_id,
                        self.mod_index,
                    ),
                    mod_index: self.mod_index,
                    event_id: id,
                    event_type: type_name,
                    client,
                    priority,
                    receive_cancelled,
                    call: Arc::from(listener.call),
                },
            );
            Ok(())
        })();
        result.inspect_err(|error| {
            self.refused.push(format!(
                "mod {} event listener refused: {error}",
                self.mod_id
            ));
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{ClientHost, Flow, Hooks, ModEntry, Registrar, RegistrationContext, ServerHost};
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    #[derive(crate::Event)]
    #[event(id = "chemlib:reaction", side = server, cancellable)]
    #[non_exhaustive]
    struct Reaction {
        heard: Vec<&'static str>,
        cancelled: bool,
    }

    #[derive(crate::Event)]
    #[event(id = "chemlib:reaction")]
    #[non_exhaustive]
    struct Impostor;

    #[derive(crate::Event)]
    #[event(id = "chemlib:frame", side = client)]
    #[non_exhaustive]
    struct Frame;

    /// Hears `Reaction` at `priority`, records `name`, and cancels it or clears the cancel.
    struct Listener {
        name: &'static str,
        cancel: bool,
        uncancel: bool,
        calls: Arc<AtomicU32>,
    }

    fn listener(name: &'static str) -> Listener {
        Listener {
            name,
            cancel: false,
            uncancel: false,
            calls: Arc::default(),
        }
    }

    impl Listener {
        fn hear(&self, _: &mut dyn ServerHost, event: &mut Reaction) -> Flow {
            self.calls.fetch_add(1, Ordering::Relaxed);
            assert_ne!(self.name, "panics", "listener panics");
            event.heard.push(self.name);
            if self.uncancel {
                event.cancelled = false;
            }
            if self.cancel {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    macro_rules! listener_mod {
        ($name:ident, $($subscribe:tt)*) => {
            struct $name(Listener);
            #[mod_api::__subscriptions]
            impl Hooks for $name {
                #[subscribe($($subscribe)*)]
                fn hear(&self, host: &mut dyn ServerHost, event: &mut Reaction) -> Flow {
                    self.0.hear(host, event)
                }
            }
        };
    }
    listener_mod!(Plain, priority = 0);
    listener_mod!(Early, priority = 5);
    listener_mod!(Late, priority = 0, receive_cancelled);

    fn post(host: &mut dyn ServerHost) -> Reaction {
        let mut event = Reaction {
            heard: Vec::new(),
            cancelled: false,
        };
        host.post(&mut event);
        event
    }

    #[test]
    fn listeners_run_by_priority_then_mod_order_and_skip_a_cancelled_event_unless_they_receive_it()
    {
        let mods: crate::Mods = [
            ModEntry::new("plain", Box::new(Plain(listener("plain")))),
            ModEntry::new(
                "early",
                Box::new(Early(Listener {
                    cancel: true,
                    ..listener("early")
                })),
            ),
            ModEntry::new("late", Box::new(Late(listener("late")))),
            ModEntry::new(
                "later",
                Box::new(Late(Listener {
                    uncancel: true,
                    ..listener("later")
                })),
            ),
        ]
        .into();
        crate::testing::with_mods(mods.clone(), || {
            crate::parts::tests::install_parts_fixture(&mods).unwrap();
            let mut host = crate::testing::FakeHost::new();
            let event = post(&mut host);
            assert_eq!(event.heard, ["early", "late", "later"]);
            assert!(!event.cancelled);

            assert!(crate::__private::set_mod_enabled("early", false));
            let event = post(&mut host);
            assert_eq!(event.heard, ["plain", "late", "later"]);
        });
    }

    #[test]
    fn a_panicking_listener_is_disabled_and_the_others_still_hear_the_event() {
        let panics = listener("panics");
        let calls = panics.calls.clone();
        let mods: crate::Mods = [
            ModEntry::new("panics", Box::new(Early(panics))),
            ModEntry::new("plain", Box::new(Plain(listener("plain")))),
        ]
        .into();
        crate::testing::with_mods(mods.clone(), || {
            crate::parts::tests::install_parts_fixture(&mods).unwrap();
            let mut host = crate::testing::FakeHost::new();
            assert_eq!(post(&mut host).heard, ["plain"]);
            assert_eq!(post(&mut host).heard, ["plain"]);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        });
    }

    #[test]
    fn a_listener_compiled_out_with_its_optional_dependency_leaves_the_mods_own_register() {
        struct Optional(Arc<AtomicU32>);
        #[mod_api::__subscriptions]
        impl Hooks for Optional {
            // As `#[cfg(feature = "chemlib")]` with chemlib absent: the method and the event
            // type it names are compiled out.
            #[cfg(any())]
            #[subscribe]
            fn hear(&self, _: &mut dyn ServerHost, _: &mut absent_dependency::Event) -> Flow {
                Flow::Continue
            }
            fn register(&self, _: &mut dyn Registrar, _: &RegistrationContext) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let registered = Arc::new(AtomicU32::new(0));
        let mods: crate::Mods = [
            ModEntry::new("optional", Box::new(Optional(registered.clone()))),
            ModEntry::new("plain", Box::new(Plain(listener("plain")))),
        ]
        .into();
        crate::testing::with_mods(mods.clone(), || {
            // `with_mods` has already run each `register` once for its own tables.
            let before = registered.load(Ordering::Relaxed);
            crate::parts::tests::install_parts_fixture(&mods).unwrap();
            assert_eq!(registered.load(Ordering::Relaxed), before + 1);
            assert_eq!(post(&mut crate::testing::FakeHost::new()).heard, ["plain"]);
        });
    }

    #[test]
    fn listening_to_one_id_with_two_types_refuses_the_second_mod() {
        struct Impostors;
        #[mod_api::__subscriptions]
        impl Hooks for Impostors {
            #[subscribe]
            fn hear(&self, _: &mut dyn crate::Host, _: &mut Impostor) {}
        }
        let mods = [
            ModEntry::new("plain", Box::new(Plain(listener("plain")))),
            ModEntry::new("impostors", Box::new(Impostors)),
        ];
        let errors = crate::parts::tests::install_parts_fixture(&mods).unwrap_err();
        let [error] = errors.as_slice() else {
            panic!("expected one refusal: {errors:?}")
        };
        assert!(
            error.contains("mod impostors")
                && error.contains("event chemlib:reaction is listened to with two types")
                && error.contains("by plain"),
            "{error}"
        );
    }

    #[test]
    fn a_listener_must_be_a_method_of_the_registering_mod() {
        struct Borrower;
        #[mod_api::__subscriptions]
        impl Hooks for Borrower {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let refused = r.add_listener(0, Plain::hear).unwrap_err();
                assert!(matches!(refused, crate::DeclarationError::Rejected { .. }));
            }
        }
        let mods = [ModEntry::new("borrower", Box::new(Borrower))];
        let errors = crate::parts::tests::install_parts_fixture(&mods).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("not a method of this mod's Hooks type")),
            "{errors:?}"
        );
    }

    #[test]
    fn a_client_event_listener_needs_the_mod_on_the_client() {
        struct Watcher;
        #[mod_api::__subscriptions]
        impl Hooks for Watcher {
            #[subscribe]
            fn frame(&self, _: &mut dyn ClientHost, _: &mut Frame) {}
        }
        let mods: crate::Mods = [
            ModEntry::new("watcher", Box::new(Watcher)),
            ModEntry::new("plain", Box::new(Plain(listener("plain")))),
        ]
        .into();
        crate::testing::with_mods(mods.clone(), || {
            crate::parts::tests::install_parts_fixture(&mods).unwrap();
            assert_eq!(
                crate::__private::client_requirement("watcher").as_deref(),
                Some("listens to the client event chemlib:frame")
            );
            assert_eq!(crate::__private::client_requirement("plain"), None);
        });
    }
}
