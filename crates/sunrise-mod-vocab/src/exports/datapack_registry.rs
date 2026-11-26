pub use world::datapack_registry::{
    BlockPredicateValue, BlockStateProviderValue, DatapackValueError, DatapackVanillaValue,
    HolderSetValue, IntProviderValue,
};
pub use world::datapack_registry::{
    DatapackDecodeError, DatapackEntry, DatapackField, DatapackFieldKind, DatapackKeyKind,
    DatapackRegistrationError, DatapackRegistryDeclaration, DatapackRegistrySync, DatapackValue,
    DatapackVariant,
};
pub use world::datapack_registry_set::{
    DatapackCandidate, DatapackLoadError, DatapackLoadReason, DatapackReadError,
    DatapackRegistrySet,
};
pub use world::datapack_snapshot::{
    DatapackNegotiationError, DatapackSnapshot, DatapackSnapshotEntry, DatapackSnapshotError,
    DatapackSnapshotRegistry, MAX_SNAPSHOT_BYTES,
};

mod __private {
    mod game {
        pub use crate::world::datapack_registry::{
            datapack_registry_directory, decode_datapack_entry, valid_namespaced,
        };
        pub use crate::world::datapack_registry_set::{
            DatapackCandidateBuilder, datapack_entry_id, datapack_tag_members,
        };
    }
    pub use crate::world::datapack_registry::{NoVanillaValueDecoder, VanillaValueDecoder};
    pub use crate::world::datapack_registry_set::{
        declarations_use_vanilla_values, queued_datapack_set_bytes, rebind_datapack_vanilla_values,
    };
}
