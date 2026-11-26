//! Load-time typed attachment registration and admission validation.
use super::*;

impl AttachmentTypesRegistrar for PartRegistrar {
    fn submit_attachment(
        &mut self,
        descriptor: crate::ErasedAttachmentDescriptor,
    ) -> Result<crate::RegisteredAttachment, String> {
        let id = descriptor.id();
        let result = (|| {
            // A new registration epoch can be collected while the previous immutable
            // snapshot remains installed. Only the live registrar scope authorizes
            // that pass; world callers have no registrar scope.
            if mod_vocab::__private::parts_installation::parts_installed()
                && !REGISTERING_KEYS.with_borrow(Option::is_some)
            {
                return Err("declarations are frozen".to_owned());
            }
            let record =
                mod_vocab::__private::check_attachment_descriptor(descriptor, self.mod_id)?;
            if let Some(existing) = self
                .attachment_types
                .get(record.namespace())
                .and_then(|types| types.get(record.name()))
            {
                return Err(format!("already declared by package {}", existing.owner()));
            }
            self.attachment_types
                .entry(record.namespace())
                .or_default()
                .insert(record.name(), record.clone());
            Ok(record)
        })();
        result.map_err(|reason| {
            let message = format!("package {} attachment {id}: {reason}", self.mod_id);
            self.refused.push(message.clone());
            message
        })
    }

    fn lookup_attachment(
        &self,
        id: &str,
    ) -> Result<crate::RegisteredAttachment, crate::AttachmentError> {
        let Some((namespace, name)) =
            crate::Identifier::parse_namespaced(id).and_then(|_| id.split_once(':'))
        else {
            return Err(crate::AttachmentError::Undeclared {
                package_id: self.mod_id.into(),
                attachment_id: id.into(),
            });
        };
        self.attachment_types
            .get(namespace)
            .and_then(|types| types.get(name))
            .cloned()
            .ok_or_else(|| crate::AttachmentError::Undeclared {
                package_id: self.mod_id.into(),
                attachment_id: id.into(),
            })
    }
}

impl PartRegistrar {
    // Declaration conflicts and stale part-state keys have already refused installation.
    // Check that no `side = "server"` package declared a synced attachment.
    pub(crate) fn validate_attachment_admission(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let packages = crate::load_state::current_packages();
        let mut synced: Vec<_> = self
            .attachment_types
            .values()
            .flat_map(|types| types.values())
            .filter(|record| record.is_synced())
            .map(|record| (record.owner(), record.namespace(), record.name()))
            .collect();
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
    use std::sync::Mutex;

    fn reg(registrar: &mut PartRegistrar) -> &mut dyn crate::Registrar {
        registrar
    }

    fn u32_codec() -> crate::AttachmentCodec<u32> {
        crate::AttachmentCodec {
            encode: |n| Ok([("n".into(), crate::Tag::Long(i64::from(*n)))].into()),
            decode: |saved| match saved.get(&"n".into()) {
                Some(crate::Tag::Long(n)) => {
                    u32::try_from(*n).map_err(|_| crate::AttachmentError::Decode)
                }
                _ => Err(crate::AttachmentError::Decode),
            },
        }
    }

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
    fn sync_without_persistence_codec_is_accepted_and_copy_on_death_without_codec_is_refused() {
        let mut registrar = PartRegistrar::new("a");
        let sync_only = reg(&mut registrar)
            .register_attachment_type(
                crate::AttachmentDescriptor::new("a:sync_only", || 7u32).sync(u32_codec()),
            )
            .expect("sync without persistence codec must be accepted");
        assert!(sync_only.is_synced());
        assert!(!sync_only.has_codec());

        assert_eq!(
            reg(&mut registrar).register_attachment_type(
                crate::AttachmentDescriptor::new("a:bad_copy", || 0u32).copy_on_death(),
            ),
            Err("package a attachment a:bad_copy: copy_on_death requires a serializer".into())
        );
    }

    #[test]
    fn non_mod_namespace_attachment_declaration_records_declaring_owner() {
        let mut registrar = PartRegistrar::new("b");
        let key = reg(&mut registrar)
            .register_attachment_type(crate::AttachmentDescriptor::new("a:mana", || 5u32))
            .unwrap();
        assert_eq!(key.id(), "a:mana");
        assert_eq!(key.owner(), "b");
        assert_eq!(registrar.attachment_types["a"]["mana"].owner(), "b");
        assert!(!registrar.attachment_types["a"]["mana"].is_synced());
        registrar.mod_id = "c";
        assert_eq!(
            reg(&mut registrar).register_attachment_type(
                crate::AttachmentDescriptor::new("a:mana", || 5u32).sync(u32_codec())
            ),
            Err("package c attachment a:mana: already declared by package b".into())
        );
    }

    #[test]
    fn server_synced_attachment_under_foreign_namespace_is_attributed_to_declarer() {
        crate::load_state::with_test_package_inventory(
            vec![
                package("a", crate::Side::Both),
                package("b", crate::Side::Server),
            ],
            || {
                let mut bad = PartRegistrar::new("b");
                reg(&mut bad)
                    .register_attachment_type(
                        crate::AttachmentDescriptor::new("a:mana", || 5u32).sync(u32_codec()),
                    )
                    .unwrap();
                assert_eq!(
                    super::super::install_registrar(bad, &crate::parts::tests::rule_schema()),
                    Err(vec![
                        "package b attachment a:mana: a side = \"server\" package cannot declare a synced attachment"
                            .into()
                    ])
                );
            },
        );
        let mut registrar = PartRegistrar::new("b");
        reg(&mut registrar)
            .register_attachment_type(
                crate::AttachmentDescriptor::new("a:mana", || 5u32).sync(u32_codec()),
            )
            .unwrap();
        super::super::install_registrar(registrar, &crate::parts::tests::rule_schema()).unwrap();
        assert_eq!(
            crate::__private::client_requirement("b"),
            Some("declares the synced attachment a:mana".into())
        );
        assert_eq!(crate::__private::client_requirement("a"), None);
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
                reg(&mut bad)
                    .register_attachment_type(crate::AttachmentDescriptor::new(
                        "test_pkg:session",
                        || 0u32,
                    ))
                    .unwrap();
                reg(&mut bad)
                    .register_attachment_type(
                        crate::AttachmentDescriptor::new("test_pkg:mana", || 0u32)
                            .sync(u32_codec()),
                    )
                    .unwrap();
                assert_eq!(
                    super::super::install_registrar(bad, &crate::parts::tests::rule_schema()),
                    Err(vec![
                        "package test_pkg attachment test_pkg:mana: a side = \"server\" package cannot declare a synced attachment".into()
                    ])
                );
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
                reg(&mut good)
                    .register_attachment_type(crate::AttachmentDescriptor::new(
                        "test_pkg:mana",
                        || 0u32,
                    ))
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
                registrar
                    .register_attachment_type(crate::AttachmentDescriptor::new("a:mana", || 37u32))
                    .unwrap();
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
        let key = sunrise_mod_runtime::attachments::AttachmentType {
            namespace: "a",
            name: "mana",
            default: || 37u32,
            codec: None,
            copy_on_death: false,
            sync_codec: None,
        };
        let mut store = Store(crate::Attachments::default());
        assert_eq!(key.get(&mut store, crate::Holder::Level), Ok(37));
        let missing = sunrise_mod_runtime::attachments::AttachmentType {
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
            Err(error)
        );
    }

    #[test]
    fn dependency_looks_up_typed_key_and_detects_type_mismatch_and_stale_key() {
        struct Owner(Arc<Mutex<Option<crate::AttachmentKey<u32>>>>);
        impl crate::Hooks for Owner {
            fn subscribed(&self) -> &'static [Option<crate::HookId>] {
                &[]
            }
            fn register(
                &self,
                registrar: &mut dyn crate::Registrar,
                _: &crate::RegistrationContext,
            ) {
                let key = registrar
                    .register_attachment_type(crate::AttachmentDescriptor::new("a:mana", || 37u32))
                    .unwrap();
                *self.0.lock().unwrap() = Some(key);
            }
        }
        struct Consumer(Arc<Mutex<Option<crate::AttachmentKey<u32>>>>);
        impl crate::Hooks for Consumer {
            fn subscribed(&self) -> &'static [Option<crate::HookId>] {
                &[]
            }
            fn register(
                &self,
                registrar: &mut dyn crate::Registrar,
                _: &crate::RegistrationContext,
            ) {
                assert_eq!(
                    registrar.attachment_key::<i64>("a:mana"),
                    Err(crate::AttachmentError::TypeMismatch)
                );
                assert_eq!(
                    registrar.attachment_key::<u32>("a:missing"),
                    Err(crate::AttachmentError::Undeclared {
                        package_id: "b".into(),
                        attachment_id: "a:missing".into(),
                    })
                );
                let key = registrar.attachment_key::<u32>("a:mana").unwrap();
                *self.0.lock().unwrap() = Some(key);
            }
        }
        let owner_key = Arc::new(Mutex::new(None));
        let consumer_key = Arc::new(Mutex::new(None));
        let packages = [
            crate::ModEntry::new("a", Box::new(Owner(Arc::clone(&owner_key)))),
            crate::ModEntry::new("b", Box::new(Consumer(Arc::clone(&consumer_key)))),
        ];
        crate::__private::install_parts(&packages, &crate::parts::tests::rule_schema(), |_| false)
            .unwrap();
        let owner_key = owner_key.lock().unwrap().take().unwrap();
        let consumer_key = consumer_key.lock().unwrap().take().unwrap();
        assert_eq!(owner_key, consumer_key);

        let mut host = crate::testing::FakeHost::default();
        assert_eq!(
            consumer_key.get(&mut host, crate::Holder::Level, Clone::clone),
            Ok(37)
        );
        let batch: crate::AttachmentBatch =
            sunrise_mod_runtime::attachments::AttachmentBatch::empty().into();
        assert!(
            batch
                .query(&consumer_key)
                .unwrap()
                .iter()
                .try_next()
                .unwrap()
                .is_none()
        );

        // Reinstall: previously issued keys become stale even with the same ID and Rust type.
        let owner_key_next = Arc::new(Mutex::new(None));
        let packages_next = [crate::ModEntry::new(
            "a",
            Box::new(Owner(Arc::clone(&owner_key_next))),
        )];
        crate::__private::install_parts(
            &packages_next,
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap();
        let owner_key_next = owner_key_next.lock().unwrap().take().unwrap();
        assert_ne!(owner_key, owner_key_next);
        assert_eq!(
            consumer_key.get(&mut host, crate::Holder::Level, Clone::clone),
            Err(crate::AttachmentError::StaleKey)
        );
        let stale_batch: crate::AttachmentBatch =
            sunrise_mod_runtime::attachments::AttachmentBatch::empty().into();
        assert_eq!(
            stale_batch.query(&consumer_key).err(),
            Some(crate::AttachmentError::StaleKey)
        );
        let live_batch: crate::AttachmentBatch =
            sunrise_mod_runtime::attachments::AttachmentBatch::empty().into();
        assert!(
            live_batch
                .query(&owner_key_next)
                .unwrap()
                .iter()
                .try_next()
                .unwrap()
                .is_none()
        );
        assert_eq!(
            owner_key_next.get(&mut host, crate::Holder::Level, Clone::clone),
            Ok(37)
        );
    }

    #[test]
    fn synced_attachment_is_named_to_server_package_admission() {
        let mut registrar = PartRegistrar::new("test_pkg");
        reg(&mut registrar)
            .register_attachment_type(
                crate::AttachmentDescriptor::new("test_pkg:mana", || 0u32).sync(u32_codec()),
            )
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
            registrar
                .register_attachment_type(
                    crate::AttachmentDescriptor::new("a:mana", || 37u32)
                        .serialize(u32_codec())
                        .sync(u32_codec()),
                )
                .unwrap();
        }
    }

    #[derive(Default)]
    struct ServerUser(Arc<Mutex<Option<crate::AttachmentKey<u32>>>>);
    impl crate::Hooks for ServerUser {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, registrar: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
            let key = registrar.attachment_key::<u32>("a:mana").unwrap();
            *self.0.lock().unwrap() = Some(key);
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
                let key_slot = Arc::new(Mutex::new(None));
                let mods = [
                    crate::ModEntry::new("a", Box::new(SyncedOwner)),
                    crate::ModEntry::new("b", Box::new(ServerUser(Arc::clone(&key_slot)))),
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
                let key = key_slot.lock().unwrap().take().unwrap();
                let mut host = crate::testing::FakeHost::default();
                assert_eq!(
                    key.get(&mut host, crate::Holder::Level, Clone::clone),
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
                    crate::ModEntry::new("b", Box::new(ServerUser::default())),
                ];
                assert_eq!(
                    crate::__private::install_parts(&mods, &crate::parts::tests::rule_schema(), |_| {
                        false
                    }),
                    Err(vec![
                        "package a attachment a:mana: a side = \"server\" package cannot declare a synced attachment".into()
                    ])
                );
                assert!(!mod_vocab::__private::parts_installation::parts_installed());
            },
        );
    }

    #[test]
    fn load_checks_conflicts_then_stale_part_state_then_server_declarations() {
        crate::load_state::with_test_package_inventory(
            vec![package("test_pkg", crate::Side::Server)],
            || {
                let mut conflict = PartRegistrar::new("test_pkg");
                reg(&mut conflict)
                    .register_attachment_type(
                        crate::AttachmentDescriptor::new("test_pkg:mana", || 0u32)
                            .sync(u32_codec()),
                    )
                    .unwrap();
                assert_eq!(
                    reg(&mut conflict).register_attachment_type(crate::AttachmentDescriptor::new(
                        "test_pkg:mana",
                        || 0u32
                    )),
                    Err(
                        "package test_pkg attachment test_pkg:mana: already declared by package test_pkg"
                            .into()
                    )
                );
                assert_eq!(
                    super::super::install_registrar(conflict, &crate::parts::tests::rule_schema()),
                    Err(vec![
                        "package test_pkg attachment test_pkg:mana: already declared by package test_pkg"
                            .into()
                    ])
                );
                let mut stale_registrar = PartRegistrar::new("test_pkg");
                reg(&mut stale_registrar)
                    .register_attachment_type(
                        crate::AttachmentDescriptor::new("test_pkg:mana", || 0u32)
                            .sync(u32_codec()),
                    )
                    .unwrap();
                let stale_key = crate::testing::registered_attachment_fixture(
                    "test_pkg",
                    crate::AttachmentDescriptor::new("test_pkg:missing", || 0u32),
                )
                .1;
                let declaration = crate::PartStateDeclaration::of(&stale_key);
                assert_eq!(
                    stale_registrar
                        .declare_block_entity_part_state("test_pkg::Part", declaration.clone()),
                    Err(declaration.refused("test_pkg::Part", crate::PartStateRefusal::StaleKey))
                );
                let prior_refusals = stale_registrar.refused.clone();
                assert_eq!(prior_refusals.len(), 1);
                assert_eq!(
                    super::super::install_registrar(
                        stale_registrar,
                        &crate::parts::tests::rule_schema()
                    ),
                    Err(prior_refusals),
                );
                assert!(!mod_vocab::__private::parts_installation::parts_installed());
            },
        );
    }

    #[test]
    fn duplicate_declaration_names_the_caller_and_original_owner() {
        let mut registrar = PartRegistrar::new("a");
        reg(&mut registrar)
            .register_attachment_type(
                crate::AttachmentDescriptor::new("a:mana", || 0u32).sync(u32_codec()),
            )
            .unwrap();
        registrar.mod_id = "b";
        assert_eq!(
            reg(&mut registrar)
                .register_attachment_type(crate::AttachmentDescriptor::new("a:mana", || 0u32)),
            Err("package b attachment a:mana: already declared by package a".into())
        );
    }

    #[test]
    fn attachment_keys_are_independent_of_namespaces_but_enforce_supplier_side() {
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
            reg(&mut bad)
                .register_attachment_type(
                    crate::AttachmentDescriptor::new("srv_runes:synced", || 0u32).sync(u32_codec()),
                )
                .unwrap();
            let errors = super::super::install_registrar(bad, &crate::parts::tests::rule_schema())
                .unwrap_err();
            assert_eq!(
                errors,
                [
                    r#"package host attachment srv_runes:synced: a side = "server" package (srv_runes) cannot supply a synced attachment"#
                ]
            );
            assert!(!mod_vocab::__private::parts_installation::parts_installed());

            let mut good = PartRegistrar::new("host");
            reg(&mut good)
                .register_attachment_type(
                    crate::AttachmentDescriptor::new("runes:charge", || 0u32).sync(u32_codec()),
                )
                .unwrap();
            reg(&mut good)
                .register_attachment_type(crate::AttachmentDescriptor::new(
                    "srv_runes:internal",
                    || 0u32,
                ))
                .unwrap();
            good.mod_id = "other";
            reg(&mut good)
                .register_attachment_type(crate::AttachmentDescriptor::new("runes:other", || 0u32))
                .unwrap();
            assert_eq!(good.attachment_types["runes"]["other"].owner(), "other");

            let mut duplicate = PartRegistrar::new("host");
            reg(&mut duplicate)
                .register_attachment_type(
                    crate::AttachmentDescriptor::new("runes:charge", || 0u32).sync(u32_codec()),
                )
                .unwrap();
            duplicate.mod_id = "other";
            let duplicate_error =
                "package other attachment runes:charge: already declared by package host";
            assert_eq!(
                reg(&mut duplicate).register_attachment_type(crate::AttachmentDescriptor::new(
                    "runes:charge",
                    || 0u32
                )),
                Err(duplicate_error.into())
            );
            assert_eq!(
                super::super::install_registrar(duplicate, &crate::parts::tests::rule_schema()),
                Err(vec![duplicate_error.into()])
            );

            crate::testing::with_mods(Vec::new().into(), || {
                super::super::install_registrar(good, &crate::parts::tests::rule_schema()).unwrap();
                assert_eq!(
                    crate::__private::client_requirement("host"),
                    Some("declares the synced attachment runes:charge".into())
                );
                assert_eq!(crate::__private::client_requirement("other"), None);
                let _caller =
                    crate::content::crash::enter_native_mod(crate::content::crash::ModCall {
                        id: "other",
                        hook: "test",
                        part: None,
                    });
                for (namespace, name, synced) in [
                    ("runes", "charge", true),
                    ("srv_runes", "internal", false),
                    ("runes", "other", false),
                ] {
                    assert_eq!(
                        mod_vocab::__private::validate_attachment_use(namespace, name, synced),
                        Ok(())
                    );
                }
            });
        });
    }
}
