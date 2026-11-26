mod stubs {
    pub(super) struct LandingWrite;
    #[derive(Default)]
    pub(super) struct InsideEffects;
    impl InsideEffects {
        pub(crate) fn stage(&mut self, _: &'static str, listen: impl FnOnce(&mut Self) -> bool) {
            let _ = listen(self);
        }
    }
}
