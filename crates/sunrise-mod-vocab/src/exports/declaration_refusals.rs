pub use crate::parts::{DeclarationError, LoadingMessage};
mod __private {
    pub use crate::parts::{
        DeclarationKind, DeclarationRefusal, DeclarationSink, DeclarationStage,
        registration_warnings, take_declaration_refusals,
    };
}
