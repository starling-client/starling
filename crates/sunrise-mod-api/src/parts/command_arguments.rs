//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl CommandArgumentsRegistrar for PartRegistrar {
    fn add_command_argument(&mut self, name: &str, argument: Arc<dyn crate::CommandArgumentType>) {
        let id = match self.declaration_id(name) {
            Ok(id) => id,
            Err(reason) => {
                self.refused.push(reason);
                return;
            }
        };
        let refusal = if self
            .parts
            .command_arguments
            .iter()
            .any(|other| *other.id == id)
        {
            Some(format!(
                "{} registered command argument type {id} twice",
                self.mod_id
            ))
        } else if self.has_builtin_entry("minecraft:command_argument_type", &id) {
            Some(format!(
                "{} registered built-in command argument type {id}",
                self.mod_id
            ))
        } else {
            None
        };
        if let Some(refusal) = refusal {
            self.refused.push(refusal);
            return;
        }
        self.parts.command_arguments.push(CommandArgumentInfo {
            id: id.into(),
            argument,
        });
    }
}

/// Fixtures that tests of other crates use to install command argument types.
#[cfg(any(test, feature = "testing"))]
pub mod tests {

    #[test]
    pub(crate) fn registration_accepts_explicit_namespaces_and_refuses_duplicates_and_builtins() {
        /// A tiny argument with two valid values.
        struct Mode;

        impl crate::CommandArgumentType for Mode {
            fn kind(&self) -> crate::CommandArgumentKind {
                crate::CommandArgumentKind::Word
            }

            fn parse(&self, input: &str) -> Result<String, crate::CommandArgumentError> {
                match input {
                    "fast" | "slow" => Ok(input.to_owned()),
                    _ => Err(crate::CommandArgumentError::new(crate::Text::translate(
                        "test.commands.mode.unknown",
                        vec![crate::Text::literal(input)],
                    ))),
                }
            }

            fn suggest(&self, request: &crate::SuggestionRequest) -> Vec<crate::Suggestion> {
                ["fast", "slow"]
                    .into_iter()
                    .filter(|mode| mode.starts_with(request.prefix.as_str()))
                    .map(|mode| request.suggest(mode))
                    .collect()
            }
        }

        struct Modes;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Modes {
            fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
                r.register_command_argument("mode", Mode);
                r.register_command_argument("other:mode", Mode);
                r.register_command_argument("minecraft:custom_mode", Mode);
            }
        }

        struct Invalid;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Invalid {
            fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
                r.register_command_argument("mode", Mode);
                r.register_command_argument("mode", Mode);
                r.register_command_argument("minecraft:entity", Mode);
                r.register_command_argument("brigadier:bool", Mode);
                r.register_command_argument("", Mode);
            }
        }

        let data = mod_vocab::__private::RegistrationData::new(
            [(
                "minecraft:command_argument_type".into(),
                vec!["brigadier:bool".into(), "minecraft:entity".into()],
            )]
            .into(),
            Default::default(),
            crate::InstanceDirectory::from_path(std::path::Path::new("unused")).unwrap(),
            crate::RuntimeEnvironment::new(
                crate::Dist::Client,
                "26.3".into(),
                "test".into(),
                false,
            ),
        );
        crate::load_state::with_test_registration_data(data, || {
            crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
                "test",
                Box::new(Modes),
            )])
            .unwrap();
            let installed = mod_vocab::__private::parts_installation::testing_command_arguments();
            assert_eq!(
                installed.iter().map(|a| &*a.id).collect::<Vec<_>>(),
                ["test:mode", "other:mode", "minecraft:custom_mode"]
            );
            assert_eq!(installed[0].argument.parse("fast"), Ok("fast".to_owned()));
            assert_eq!(
                installed[0]
                    .argument
                    .suggest(&crate::SuggestionRequest::new("s", Vec::new(), None)),
                [crate::Suggestion::new((0, 1), "slow")]
            );

            let errors = crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
                "test",
                Box::new(Invalid),
            )])
            .unwrap_err();
            assert_eq!(errors.len(), 4, "{errors:?}");
            assert!(errors[0].contains("test registered command argument type test:mode twice"));
            assert!(
                errors[1]
                    .contains("test registered built-in command argument type minecraft:entity")
            );
            assert!(
                errors[2].contains("test registered built-in command argument type brigadier:bool")
            );
            assert!(errors[3].contains("invalid identifier \"test:\""));
        });
    }
}
