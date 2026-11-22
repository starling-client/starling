//! Declaration dispatch passes the owner and context to each mod's registrar view.
mod stubs {
    pub(super) mod declaration_ownership {
        use crate::tests::{Answered, DeclarationError, RegistrationContext, mod_api};

        pub(in crate::tests) trait Declaration {
            fn id(&mut self) -> &mut String;
        }
        impl Declaration for mod_api::mod_api::Block {
            fn id(&mut self) -> &mut String {
                &mut self.id
            }
        }
        impl Declaration for mod_api::mod_api::Item {
            fn id(&mut self) -> &mut String {
                &mut self.id
            }
        }
        pub(in crate::tests) type DeclarationRefusal = ();
        pub(in crate::tests) type DeclarationSink<'a, D> =
            dyn FnMut(Answered<D>) -> Result<(), DeclarationError> + 'a;
        pub(in crate::tests) type DeclarationStage<'a, D> =
            dyn FnMut(&mut dyn FnMut(&mut DeclarationSink<'_, D>) -> bool) + 'a;
        pub(in crate::tests) fn declaration_listener(
            _: &RegistrationContext,
            run: impl FnOnce(&mut Vec<DeclarationRefusal>) -> bool,
        ) -> bool {
            run(&mut Vec::new())
        }
        pub(in crate::tests) fn submit_declaration<T: Declaration>(
            _: &mut Vec<DeclarationRefusal>,
            mod_id: &'static str,
            output: &mut dyn FnMut(Answered<T>) -> Result<(), DeclarationError>,
            mut answer: T,
        ) -> Result<(), DeclarationError> {
            let id = answer.id();
            crate::tests::DECLARATION_OWNERS.with_borrow_mut(|calls| {
                calls.push((mod_id.to_owned(), id.clone()));
            });
            if id == "reject" {
                return Err(DeclarationError::Rejected {
                    id: id.clone(),
                    reason: "bad ID".into(),
                });
            }
            *id = format!("{mod_id}/checked/{id}");
            output(Answered { answer, mod_id })
        }
    }
}
use stubs::declaration_ownership as parts;

thread_local! {
    static DECLARATION_OWNERS: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[test]
fn block_dispatch_keeps_owner_context_and_valid_output() {
    struct Declares(&'static str);
    #[mod_api::__subscriptions]
    impl Hooks for Declares {
        fn blocks(&self, context: &RegistrationContext, event: &mut BlocksEvent<'_>) {
            assert_eq!(context.0, "bootstrap");
            let result = event.register(mod_api::mod_api::Block { id: self.0.into() });
            if self.0 == "reject" {
                assert!(
                    matches!(result, Err(DeclarationError::Rejected { id, reason }) if id == "reject" && reason == "bad ID")
                );
            } else {
                result.unwrap();
            }
        }
    }
    install(
        vec![
            ModEntry::new("first", Box::new(Declares("first:pot"))),
            ModEntry::new("second", Box::new(Declares("reject"))),
        ]
        .into(),
    );
    DECLARATION_OWNERS.with_borrow_mut(Vec::clear);
    let mut blocks = vec![Answered {
        answer: mod_api::mod_api::Block {
            id: "existing".into(),
        },
        mod_id: "existing",
    }];
    dispatch_blocks(&RegistrationContext("bootstrap"), &mut |listen| {
        assert!(listen(&mut |block| {
            blocks.push(block);
            Ok(())
        }));
    });
    assert_eq!(
        blocks
            .iter()
            .map(|b| (b.mod_id, b.answer.id.as_str()))
            .collect::<Vec<_>>(),
        [
            ("existing", "existing"),
            ("first", "first/checked/first:pot")
        ]
    );
    DECLARATION_OWNERS.with_borrow(|calls| {
        assert_eq!(
            calls,
            &[
                ("first".into(), "first:pot".into()),
                ("second".into(), "reject".into())
            ]
        );
    });
}
