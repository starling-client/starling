#[path = "text_style_capture.rs"]
mod text_style_capture;
use super::{TryIntoApi, TryIntoEngine};
use mod_api::text::{Text, TextFormError};
use sunrise_client_core::text as engine;

impl TryIntoApi for engine::Text {
    type Api = Text;
    type Error = TextFormError;
    fn try_into_api(&self) -> Result<Text, TextFormError> {
        sunrise_client_lib::api_text::to_api(self)
    }
}
impl<C: ?Sized> TryIntoEngine<C> for Text {
    type Engine = engine::Text;
    type Error = TextFormError;
    fn try_into_engine(&self, _: &C) -> Result<engine::Text, TextFormError> {
        sunrise_client_lib::api_text::to_engine(self)
    }
}
