pub trait TryIntoApi {
    type Api;
    type Error;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error>;
}
pub trait TryIntoEngine<C: ?Sized> {
    type Engine;
    type Error;
    fn try_into_engine(&self, context: &C) -> Result<Self::Engine, Self::Error>;
}
#[path = "../../../../crates/horizon-client/src/game/modding/api_values/damage.rs"]
mod boundary;
