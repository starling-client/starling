//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

/// The ticket controllers `mods` register, for a test's mods.
#[cfg(any(test, feature = "testing"))]
pub(crate) fn registered_controllers(mods: &[ModEntry]) -> (Vec<ControllerInfo>, Vec<PoseInfo>) {
    let registered = register_all(mods, |_| false);
    (registered.controllers, registered.arm_poses)
}

/// Fixtures that tests of other crates use to install Parts.
#[cfg(any(test, feature = "testing"))]
pub mod tests {
    #[cfg(test)]
    use super::*;

    #[cfg(test)]
    fn noop_validator(_: &mut dyn ServerHost, _: LevelRef, _: &mut dyn TicketValidation) {}

    #[test]
    pub(crate) fn ticket_controllers_keep_explicit_namespaces_and_attribute_the_caller() {
        let mut r = super::PartRegistrar::new("first");
        let quarry = r.register_ticket_controller("quarry", Some(Arc::new(noop_validator)));
        let drill = r.register_ticket_controller("first:drill", None);
        r.register_ticket_controller("second:quarry", Some(Arc::new(noop_validator)));
        r.register_ticket_controller("minecraft:custom_quarry", None);
        assert!(r.refused.is_empty(), "{:?}", r.refused);
        assert_ne!(quarry, drill);
        assert_ne!(r.controllers[0].token, 0);
        assert_ne!(r.controllers[1].token, 0);
        assert_ne!(r.controllers[0].token, r.controllers[1].token);
        let ids: Vec<_> = r
            .controllers
            .iter()
            .map(|c| (&*c.id, c.validator.is_some()))
            .collect();
        assert_eq!(
            ids,
            [
                ("first:quarry", true),
                ("first:drill", false),
                ("second:quarry", true),
                ("minecraft:custom_quarry", false),
            ]
        );
        let refused = mod_vocab::__private::parts_installation::refused_ticket_controller();
        for name in ["first:", ""] {
            assert_eq!(r.register_ticket_controller(name, None), refused);
        }
        assert_eq!(r.refused.len(), 2, "{:?}", r.refused);
        assert_eq!(r.controllers.len(), 4);
    }

    #[test]
    fn installed_ticket_controllers_read_the_installed_table() {
        let mut other = PartRegistrar::new("first");
        let foreign = other.register_ticket_controller("quarry", Some(Arc::new(noop_validator)));
        let mut registrar = PartRegistrar::new("first");
        let active = registrar.register_ticket_controller("quarry", Some(Arc::new(noop_validator)));
        install_registrar(registrar, &crate::parts::tests::rule_schema()).unwrap();
        let controllers = mod_vocab::__private::parts_installation::testing_controllers();
        let [(id, token, has_validator)] = controllers.as_slice() else {
            panic!("expected one installed controller, got {controllers:?}");
        };
        assert_eq!(*id, "first:quarry");
        assert_ne!(*token, 0);
        assert!(*has_validator);
        assert_eq!(active.id(), "first:quarry");
        assert_eq!(
            mod_vocab::__private::game::ticket_controller_slot(active),
            Some(0)
        );
        // Same slot and same namespaced id from another registration table cannot match.
        assert_ne!(foreign, active);
        assert_eq!(foreign.id(), "");
        assert_eq!(
            mod_vocab::__private::game::ticket_controller_slot(foreign),
            None
        );
    }

    /// A refused registration retires its validator inside the guard: a destructor that panics
    /// neither unwinds out of registration nor hides the refusal.
    #[test]
    fn refusing_validators_with_panicking_destructors_still_reports_the_refusal() {
        struct PanicsOnDrop;
        impl Drop for PanicsOnDrop {
            fn drop(&mut self) {
                if !std::thread::panicking() {
                    panic!("fixture destructor");
                }
            }
        }
        let validator = || -> Arc<LoadingValidationCallback> {
            let captured = PanicsOnDrop;
            Arc::new(
                move |_: &mut dyn ServerHost, _: LevelRef, _: &mut dyn TicketValidation| {
                    let _ = &captured;
                },
            )
        };
        let mut r = super::PartRegistrar::new("first");
        let kept = r.register_ticket_controller("quarry", None);
        let refused = mod_vocab::__private::parts_installation::refused_ticket_controller();
        assert_ne!(kept, refused);
        assert_eq!(
            r.register_ticket_controller("first:quarry", Some(validator())),
            refused
        );
        assert_eq!(
            r.register_ticket_controller("first:", Some(validator())),
            refused
        );
        assert_eq!(r.refused.len(), 2, "{:?}", r.refused);
        assert_eq!(r.controllers.len(), 1);
    }

    #[test]
    pub(crate) fn a_ticket_controller_registered_twice_refuses_the_mods_naming_mod_and_id() {
        let mut r = super::PartRegistrar::new("first");
        let first = r.register_ticket_controller("quarry", Some(Arc::new(noop_validator)));
        let duplicate = r.register_ticket_controller("first:quarry", None);
        assert_eq!(
            duplicate,
            mod_vocab::__private::parts_installation::refused_ticket_controller()
        );
        r.mod_id = "second";
        // A bare name under another mod is another id; reusing the first key is a duplicate.
        let second = r.register_ticket_controller("quarry", None);
        assert_ne!(first, second);
        assert_eq!(
            r.register_ticket_controller("first:quarry", None),
            duplicate
        );
        assert_eq!(r.refused.len(), 2, "{:?}", r.refused);
        assert_eq!(r.controllers.len(), 2);
        assert_eq!(&*r.controllers[1].id, "second:quarry");
        assert!(r.controllers[0].validator.is_some());
        assert!(r.controllers[1].validator.is_none());
    }
}

impl TicketsRegistrar for PartRegistrar {
    fn register_ticket_controller(
        &mut self,
        name: &str,
        validator: Option<Arc<LoadingValidationCallback>>,
    ) -> TicketController {
        register(
            &mut self.controllers,
            &mut self.refused,
            self.mod_id,
            self.mod_index,
            name,
            validator,
        )
    }
}

/// Adds controller `name` of mod `mod_id` to `table`. A malformed or repeated id is written to
/// `refused` and gives a handle no table resolves.
pub(crate) fn register(
    table: &mut Vec<ControllerInfo>,
    refused: &mut Vec<String>,
    mod_id: &'static str,
    mod_index: usize,
    name: &str,
    validator: Option<Arc<LoadingValidationCallback>>,
) -> TicketController {
    let id = match declaration_id(mod_id, name) {
        Ok(id) => id,
        Err(reason) => {
            refused.push(reason);
            mod_vocab::__private::parts_installation::dispose_refused_ticket_validator(
                mod_id, mod_index, name, validator,
            );
            return mod_vocab::__private::parts_installation::refused_ticket_controller();
        }
    };
    let refusal = {
        table
            .iter()
            .any(|info| *info.id == *id)
            .then(|| format!("{mod_id} registered ticket controller {id} twice"))
    };
    if let Some(refusal) = refusal {
        refused.push(refusal);
        mod_vocab::__private::parts_installation::dispose_refused_ticket_validator(
            mod_id, mod_index, &id, validator,
        );
        return mod_vocab::__private::parts_installation::refused_ticket_controller();
    }
    let slot = table.len() as u32;
    let (info, handle) = mod_vocab::__private::parts_installation::register_ticket_controller(
        slot,
        id.into(),
        mod_id,
        mod_index,
        validator,
    );
    table.push(info);
    handle
}
