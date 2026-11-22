use super::*;

impl TagsRegistrar for PartRegistrar {
    fn tag_key(&mut self, id: &str) -> crate::TagKey {
        let reason =
            match crate::Identifier::parse(id).map(|name| mod_vocab::__private::tag_key(&name)) {
                Ok(Some(key)) => return key,
                // No test reaches this: the name table refuses only when its lock is poisoned
                // or its slot space is exhausted. It stays because `tag_slot` answers `None`
                // there and a mod's load must be refused with a reason, never a panic.
                Ok(None) => format!("could not allocate a name slot for tag {id:?}"),
                Err(error) => format!("invalid tag name {id:?}: {error}"),
            };
        self.refused.push(format!("mod {}: {reason}", self.mod_id));
        mod_vocab::__private::parts_installation::invalid_tag_key()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Hooks, ModEntry, RegistrationContext};

    #[test]
    fn invalid_tag_name_refuses_load_with_mod_and_name() {
        struct Invalid;
        #[mod_api::__subscriptions]
        impl Hooks for Invalid {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.tag_key("test:has space");
            }
        }
        let errors = install_parts(
            &[ModEntry::new("tag_reader", Box::new(Invalid))],
            &crate::parts::tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("mod tag_reader") && errors[0].contains("test:has space"),
            "{errors:?}"
        );
    }
}
