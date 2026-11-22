pub mod __private {
    #[derive(Clone, Copy)]
    pub struct SelectedBlockScreenEffectRenderer;
    pub use super::ConfigurationTaskRegistration;
    pub trait DebugEntryStates {
        fn is_enabled(&self, id: &str) -> bool;
    }
}
