pub use item::item_model_providers::{
    ItemModelChildren, ItemModelConfig, ItemModelDecoder, ItemModelFn, ItemModelPropertyInput,
    ItemModelUpdateInput, ItemTintInput, ItemTintSourceDecoder, ItemTintSourceFn,
};
use item::item_model_providers::{
    ItemModelRegistration, ItemTintSourceRegistration, RangeSelectPropertyRegistration,
    SelectPropertyRegistration,
};
pub use item::item_model_providers::{
    RangeSelectPropertyDecoder, RangeSelectPropertyFn, SelectCaseDecoder, SelectCaseKey,
    SelectPropertyDecoder, SelectPropertyFn,
};
mod __private {
    mod game {
        pub use crate::item::item_model_providers::{
            ItemModelRegistration, ItemTintSourceRegistration, RangeSelectPropertyRegistration,
            SelectPropertyRegistration, new_item_model_config,
        };
    }
}
