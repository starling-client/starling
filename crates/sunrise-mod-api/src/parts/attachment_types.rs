//! Load-time attachment declarations, before any registration is installed.
use super::*;

struct LoadedAttachmentDeclarations;
impl crate::AttachmentDeclarations for LoadedAttachmentDeclarations {
    fn validate_attachment_type(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), crate::AttachmentError> {
        mod_vocab::__private::validate_attachment_use(namespace, name, sync)
    }
}

/// The immutable attachment type registry, with no world borrow.
pub fn attachment_declarations() -> &'static dyn crate::AttachmentDeclarations {
    static LOOKUP: LoadedAttachmentDeclarations = LoadedAttachmentDeclarations;
    &LOOKUP
}

impl AttachmentTypesRegistrar for PartRegistrar {
    fn register_attachment_type(&mut self, id: &str, has_sync_codec: bool) -> Result<(), String> {
        let result = (|| {
            // A new registration epoch can be collected while the previous immutable
            // snapshot remains installed. Only the live registrar scope authorizes
            // that pass; world callers have no registrar scope.
            if mod_vocab::__private::parts_installation::parts_installed()
                && !REGISTERING_KEYS.with_borrow(Option::is_some)
            {
                return Err("declarations are frozen".to_owned());
            }
            let identifier = crate::Identifier::parse_namespaced(id)
                .ok_or("explicit lowercase namespaced id required")?;
            if let Some(owner) = self
                .attachment_owners
                .get(identifier.namespace())
                .and_then(|types| types.get(identifier.path()))
            {
                return Err(format!("already declared by package {owner}"));
            }
            if identifier.namespace() == "minecraft"
                || !owns_namespace(self.mod_id, identifier.namespace())
            {
                return Err(format!(
                    "namespace belongs to package {}, not {}",
                    identifier.namespace(),
                    self.mod_id
                ));
            }
            self.attachment_types
                .entry(identifier.namespace().into())
                .or_default()
                .insert(identifier.path().into(), has_sync_codec);
            self.attachment_owners
                .entry(identifier.namespace().into())
                .or_default()
                .insert(identifier.path().into(), self.mod_id);
            Ok(())
        })();
        result.map_err(|reason| {
            let message = format!("package {} attachment {id}: {reason}", self.mod_id);
            self.refused.push(message.clone());
            message
        })
    }
}

impl PartRegistrar {
    // Declaration conflicts have already refused installation. Check all load-known
    // uses before considering the side of the package that owns each declaration.
    pub(crate) fn validate_attachment_admission(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        for (caller, namespace, name) in &self.attachment_uses {
            if mod_vocab::__private::validate_declared_attachment(
                Some(&self.attachment_types),
                caller,
                namespace,
                name,
                false,
            )
            .is_err()
            {
                errors.push(format!(
                    "package {caller} attachment {namespace}:{name}: undeclared attachment use"
                ));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let packages = crate::load_state::current_packages();
        let mut synced = Vec::new();
        for (namespace, types) in &self.attachment_types {
            for (name, &sync) in types {
                if !sync {
                    continue;
                }
                let declarer = self
                    .attachment_owners
                    .get(namespace)
                    .and_then(|owners| owners.get(name))
                    .copied()
                    .unwrap_or(namespace.as_str());
                synced.push((declarer, namespace.as_str(), name.as_str()));
            }
        }
        synced.sort_unstable();
        for (declarer, namespace, name) in synced {
            let declarer_server = packages
                .iter()
                .any(|p| p.id() == declarer && p.side() == crate::Side::Server);
            if declarer_server {
                errors.push(format!(
                    "package {declarer} attachment {namespace}:{name}: a side = \"server\" package cannot declare a synced attachment"
                ));
                continue;
            }
            if let Some(supplier) = packages.iter().find(|p| {
                p.side() == crate::Side::Server
                    && (p.id() == declarer || p.served_by() == Some(declarer))
                    && p.namespaces().any(|owned| owned == namespace)
            }) {
                errors.push(format!(
                    "package {declarer} attachment {namespace}:{name}: a side = \"server\" package ({}) cannot supply a synced attachment",
                    supplier.id()
                ));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Store(crate::Attachments);
    impl crate::AttachmentHost for Store {
        fn validate_attachment(
            &self,
            namespace: &str,
            name: &str,
            sync: bool,
        ) -> Result<(), crate::AttachmentError> {
            mod_vocab::__private::validate_attachment_use(namespace, name, sync)
        }
        fn attachments(
            &mut self,
            _: crate::Holder,
            _: crate::AttachmentAccess,
        ) -> Result<&mut crate::Attachments, crate::AttachmentError> {
            Ok(&mut self.0)
        }
    }
    #[test]
    fn foreign_attachment_declaration_is_named_and_not_added() {
        let mut registrar = PartRegistrar::new("b");
        assert_eq!(
            registrar.register_attachment_type("a:mana", false),
            Err("package b attachment a:mana: namespace belongs to package a, not b".into())
        );
        assert!(registrar.attachment_types.is_empty());
    }
    #[test]
    fn server_synced_attachment_refuses_all_registration_and_plain_is_admitted() {
        crate::load_state::with_test_package_inventory(
            vec![crate::PackageInfo::new(
                "test_pkg".into(),
                "1".into(),
                crate::PackageKind::Code,
                crate::Side::Server,
                vec!["test_pkg".into()],
                vec![],
            )],
            || {
                let mut bad = PartRegistrar::new("test_pkg");
                bad.register_custom_stat("marker").unwrap();
                bad.register_attachment_type("test_pkg:session", false)
                    .unwrap();
                bad.register_attachment_type("test_pkg:mana", true).unwrap();
                assert_eq!(super::super::install_registrar(bad, &crate::parts::tests::rule_schema()), Err(vec!["package test_pkg attachment test_pkg:mana: a side = \"server\" package cannot declare a synced attachment".into()]));
                assert!(mod_vocab::__private::custom_stat_declarations().is_empty());
                assert!(!mod_vocab::__private::parts_installation::parts_installed());
                let _caller =
                    crate::content::crash::enter_native_mod(crate::content::crash::ModCall {
                        id: "test_pkg",
                        hook: "test",
                        part: None,
                    });
                assert!(
                    mod_vocab::__private::validate_attachment_use("test_pkg", "session", false)
                        .is_err()
                );
                let mut good = PartRegistrar::new("test_pkg");
                good.register_attachment_type("test_pkg:mana", false)
                    .unwrap();
                super::super::install_registrar(good, &crate::parts::tests::rule_schema()).unwrap();
                assert_eq!(
                    mod_vocab::__private::validate_attachment_use("test_pkg", "mana", false),
                    Ok(())
                );
            },
        );
    }

    #[test]
    fn package_b_reads_a_declared_mana_and_undeclared_use_names_b() {
        struct Owner;
        impl crate::Hooks for Owner {
            fn subscribed(&self) -> &'static [Option<crate::HookId>] {
                &[]
            }
            fn register(
                &self,
                registrar: &mut dyn crate::Registrar,
                _: &crate::RegistrationContext,
            ) {
                registrar.register_attachment_type("a:mana", false).unwrap();
            }
        }
        let packages = [crate::ModEntry::new("a", Box::new(Owner))];
        crate::__private::install_parts(&packages, &crate::parts::tests::rule_schema(), |_| false)
            .unwrap();
        let _caller = crate::content::crash::enter_native_mod(crate::content::crash::ModCall {
            id: "b",
            hook: "test",
            part: None,
        });
        let key = crate::AttachmentType {
            namespace: "a",
            name: "mana",
            default: || 37u32,
            codec: None,
            copy_on_death: false,
            sync: false,
        };
        let mut store = Store(crate::Attachments::default());
        assert_eq!(key.get(&mut store, crate::Holder::Level), Ok(37));
        let missing = crate::AttachmentType {
            namespace: "test_pkg",
            ..key
        };
        let error = crate::AttachmentError::Undeclared {
            package_id: "b".into(),
            attachment_id: "test_pkg:mana".into(),
        };
        assert_eq!(
            missing.get(&mut store, crate::Holder::Level),
            Err(error.clone())
        );
        assert_eq!(
            missing.sync_data(&mut store, crate::Holder::Level),
            Err(error.clone())
        );
        let batch: crate::AttachmentBatch =
            sunrise_mod_runtime::attachments::AttachmentBatch::empty().into();
        assert_eq!(batch.query(&missing).err(), Some(error));
    }
    #[test]
    fn synced_attachment_is_named_to_server_package_admission() {
        let mut registrar = PartRegistrar::new("test_pkg");
        registrar
            .register_attachment_type("test_pkg:mana", true)
            .unwrap();
        super::super::install_registrar(registrar, &crate::parts::tests::rule_schema()).unwrap();
        assert_eq!(
            crate::__private::client_requirement("test_pkg"),
            Some("declares the synced attachment test_pkg:mana".into())
        );
    }
    fn package(id: &str, side: crate::Side) -> crate::PackageInfo {
        crate::PackageInfo::new(
            id.into(),
            "1".into(),
            crate::PackageKind::Code,
            side,
            vec![id.into()],
            vec![],
        )
    }
    struct SyncedOwner;
    impl crate::Hooks for SyncedOwner {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, registrar: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
            registrar.register_attachment_type("a:mana", true).unwrap();
        }
    }
    struct ServerUser;
    impl crate::Hooks for ServerUser {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
    }
    #[test]
    fn server_b_loads_and_uses_a_synced_mana() {
        crate::load_state::with_test_package_inventory(
            vec![
                package("a", crate::Side::Both),
                package("b", crate::Side::Server),
            ],
            || {
                let mods = [
                    crate::ModEntry::new("a", Box::new(SyncedOwner)),
                    crate::ModEntry::new("b", Box::new(ServerUser)),
                ];
                crate::__private::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| {
                    false
                })
                .unwrap();
                assert_eq!(crate::__private::client_requirement("b"), None);
                let _caller =
                    crate::content::crash::enter_native_mod(crate::content::crash::ModCall {
                        id: "b",
                        hook: "test",
                        part: None,
                    });
                let key = crate::AttachmentType {
                    namespace: "a",
                    name: "mana",
                    default: || 37u32,
                    codec: Some(crate::AttachmentCodec {
                        encode: |n| Ok([("n".into(), crate::Tag::Long(i64::from(*n)))].into()),
                        decode: |saved| match saved.get(&"n".into()) {
                            Some(crate::Tag::Long(n)) => {
                                u32::try_from(*n).map_err(|_| crate::AttachmentError::Decode)
                            }
                            _ => Err(crate::AttachmentError::Decode),
                        },
                    }),
                    copy_on_death: false,
                    sync: true,
                };
                assert_eq!(
                    key.get(
                        &mut Store(crate::Attachments::default()),
                        crate::Holder::Level
                    ),
                    Ok(37)
                );
            },
        );
    }
    #[test]
    fn server_declarer_a_is_refused_instead_of_server_user_b() {
        crate::load_state::with_test_package_inventory(
            vec![
                package("a", crate::Side::Server),
                package("b", crate::Side::Server),
            ],
            || {
                let mods = [
                    crate::ModEntry::new("a", Box::new(SyncedOwner)),
                    crate::ModEntry::new("b", Box::new(ServerUser)),
                ];
                assert_eq!(crate::__private::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false), Err(vec!["package a attachment a:mana: a side = \"server\" package cannot declare a synced attachment".into()]));
                assert!(!mod_vocab::__private::parts_installation::parts_installed());
            },
        );
    }
    #[test]
    fn load_checks_conflicts_then_undeclared_uses_then_server_declarations() {
        crate::load_state::with_test_package_inventory(
            vec![package("test_pkg", crate::Side::Server)],
            || {
                let mut conflict = PartRegistrar::new("test_pkg");
                conflict
                    .register_attachment_type("test_pkg:mana", true)
                    .unwrap();
                assert_eq!(conflict.register_attachment_type("a:mana", false), Err("package test_pkg attachment a:mana: namespace belongs to package a, not test_pkg".into()));
                assert_eq!(super::super::install_registrar(conflict, &crate::parts::tests::rule_schema()), Err(vec!["package test_pkg attachment a:mana: namespace belongs to package a, not test_pkg".into()]));
                let mut missing = PartRegistrar::new("test_pkg");
                missing
                    .register_attachment_type("test_pkg:mana", true)
                    .unwrap();
                let key = crate::AttachmentType {
                    namespace: "test_pkg",
                    name: "missing",
                    default: || 0u32,
                    codec: None,
                    copy_on_death: false,
                    sync: false,
                };
                missing
                    .declare_block_entity_part_state(
                        "test_pkg::Part",
                        crate::PartStateDeclaration::of(&key),
                    )
                    .unwrap();
                assert_eq!(
                    super::super::install_registrar(missing, &crate::parts::tests::rule_schema()),
                    Err(vec![
                        "package test_pkg attachment test_pkg:missing: undeclared attachment use"
                            .into()
                    ])
                );
                assert!(!mod_vocab::__private::parts_installation::parts_installed());
            },
        );
    }
    #[test]
    fn duplicate_declaration_names_the_caller_and_original_owner() {
        let mut registrar = PartRegistrar::new("a");
        registrar.register_attachment_type("a:mana", true).unwrap();
        registrar.mod_id = "b";
        assert_eq!(
            registrar.register_attachment_type("a:mana", false),
            Err("package b attachment a:mana: already declared by package a".into())
        );
    }

    #[test]
    fn served_by_attachment_delegates_and_enforces_supplier_side() {
        let packages = vec![
            package("host", crate::Side::Both),
            package("other", crate::Side::Both),
            crate::PackageInfo::new(
                "runes".into(),
                "1".into(),
                crate::PackageKind::Content,
                crate::Side::Both,
                vec!["runes".into()],
                vec![],
            )
            .with_served_by(Some("host".into())),
            crate::PackageInfo::new(
                "srv_runes".into(),
                "1".into(),
                crate::PackageKind::Content,
                crate::Side::Server,
                vec!["srv_runes".into()],
                vec![],
            )
            .with_served_by(Some("host".into())),
        ];
        crate::load_state::with_test_package_inventory(packages, || {
            // A both-side host cannot declare a synced attachment in a server-only served namespace.
            let mut bad = PartRegistrar::new("host");
            bad.register_attachment_type("srv_runes:synced", true)
                .unwrap();
            let errors = super::super::install_registrar(bad, &crate::parts::tests::rule_schema())
                .unwrap_err();
            assert_eq!(errors.len(), 1);
            assert!(
                errors[0].contains("srv_runes:synced") && errors[0].contains("side = \"server\""),
                "{errors:?}"
            );
            assert!(!mod_vocab::__private::parts_installation::parts_installed());

            let mut good = PartRegistrar::new("host");
            good.register_attachment_type("runes:charge", true).unwrap();
            good.register_attachment_type("srv_runes:internal", false)
                .unwrap();
            good.mod_id = "other";
            assert!(good.register_attachment_type("runes:other", false).is_err());
            assert_eq!(
                good.register_attachment_type("runes:charge", false),
                Err(
                    "package other attachment runes:charge: already declared by package host"
                        .into()
                )
            );
            good.refused.clear();
            super::super::install_registrar(good, &crate::parts::tests::rule_schema()).unwrap();
            assert_eq!(
                crate::__private::client_requirement("host"),
                Some("declares the synced attachment runes:charge".into())
            );
            let _caller = crate::content::crash::enter_native_mod(crate::content::crash::ModCall {
                id: "other",
                hook: "test",
                part: None,
            });
            assert_eq!(
                mod_vocab::__private::validate_attachment_use("runes", "charge", true),
                Ok(())
            );
            assert_eq!(
                mod_vocab::__private::validate_attachment_use("srv_runes", "internal", false),
                Ok(())
            );
        });
    }
}
