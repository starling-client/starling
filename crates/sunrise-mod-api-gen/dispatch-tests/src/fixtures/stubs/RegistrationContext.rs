pub(super) struct RegistrationContext(pub &'static str);

impl RegistrationContext {
    pub(super) fn for_mod(&self, _: &'static str) -> Self {
        Self(self.0)
    }
}
