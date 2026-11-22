//! The general entity renderer used by the integrated scene.
use mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, BlockEntityDrawContext, EntityRenderState,
    EntityRenderer, EntityRenderers, GeneralEntityCategory, GeneralSavePolicy, Registrar, Tag,
};

pub const KIND: &str = "general_render_fixture:render_probe";

pub static FUEL: AttachmentType<i32> = AttachmentType {
    namespace: "general_render_fixture",
    name: "render_fuel",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |tag| match tag.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: true,
};

pub fn register(registrar: &mut dyn Registrar) {
    registrar
        .register_attachment_type("general_render_fixture:render_fuel", true)
        .unwrap();
    registrar
        .declare_general_entity(
            "render_probe",
            GeneralEntityCategory::Misc,
            0.5,
            0.5,
            None,
            GeneralSavePolicy::Save,
            true,
            false,
            None,
            8,
            1,
            1,
            &[],
        )
        .unwrap();
}

pub fn register_renderer(renderers: &mut dyn EntityRenderers) {
    renderers.register(KIND.into(), std::sync::Arc::new(Probe));
}

struct Probe;
impl EntityRenderer for Probe {
    fn submit(
        &self,
        entity: &EntityRenderState,
        draw: &mut dyn BlockEntityDrawContext,
    ) -> Result<(), String> {
        let fuel = entity
            .attachments
            .as_ref()
            .ok_or("missing render snapshot")?
            .get(&FUEL)
            .map_err(|e| format!("{e:?}"))?
            .copied()
            .ok_or("missing fuel")? as f32;
        draw.quad(
            "block/stone",
            [
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0],
                [fuel, 0.0, 0.0],
                [fuel, 1.0, 0.0],
            ],
            [0.0, 0.0, 16.0, 16.0],
            0xFF_FFFF,
        );
        Ok(())
    }
}
