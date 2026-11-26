pub use world::structure_processor::{
    ProcessorContext, ProcessorDecodeContext, ProcessorFinalizeContext, StructureBlockInfo,
    StructurePlaceSettings, StructureProcessor, StructureProcessorListRef, StructureProcessorRef,
    StructureProcessorType,
};
mod __private {
    /// The registered structure processor types, and the handles the game's
    /// adapter builds and reads back.
    pub use crate::world::structure_processor::{
        structure_processor_list_payload, structure_processor_list_ref,
        structure_processor_payload, structure_processor_ref,
        structure_processor_type_declarations,
    };
}
