//! Compare the saved and first-packet light of a column whose lower pad is
//! allocated before or after its lowest block section's saved sky array arrives.
use std::{fmt::Write, path::PathBuf};
use sunrise_client::{
    assets::Assets,
    biome::Biomes,
    chunk::{Chunk, MIN_Y, SECTIONS},
    lan::chunks::ChunkEncoder,
    light::light_from_saved_layers,
    save,
    world::World,
};
use sunrise_core::nbt::Tag;
use sunrise_protocol::{
    buf::{Codec, Reader},
    chunk::ChunkData,
    ids::play::clientbound::LEVEL_CHUNK_WITH_LIGHT,
    registry::Builtin,
};
use sunrise_world::registry::BlockStateId;

fn append_hex(report: &mut String, bytes: &[u8]) -> std::fmt::Result {
    for byte in bytes {
        write!(report, "{byte:02x}")?;
    }
    writeln!(report)
}

pub(super) fn write_report(resources: PathBuf, output: PathBuf) -> sunrise_client::Result<()> {
    let assets = Assets::load(&resources)?;
    let biomes = Biomes::load(&sunrise_assets::resource_source::ResourceRoot::directory(
        resources,
    ))?;
    let items = Default::default();
    let encoder = ChunkEncoder::new(
        [("minecraft:air".into(), false)],
        &Builtin::default(),
        &[],
        &[],
        std::sync::Arc::default(),
    );
    let mut report = String::new();
    for (order, array_first) in [
        ("status-before-array", false),
        ("array-before-status", true),
    ] {
        let mut world = World::new(BlockStateId(0), 0);
        let mut upper_neighbour = Chunk::empty(1, 0);
        upper_neighbour.add_light_support(1);
        world.install_light_column(upper_neighbour);
        let install_lower_neighbour = |world: &mut World| {
            let mut neighbour = Chunk::empty(0, 1);
            neighbour.add_light_support(0);
            world.install_light_column(neighbour);
        };
        if !array_first {
            install_lower_neighbour(&mut world);
        }
        let mut chunk = Chunk::empty(0, 0);
        let mut layers = vec![(None, None); SECTIONS + 2];
        let mut sky = [0; 2048];
        sky[128] = 15; // (0, 1, 0), leaving the first row zero.
        layers[1].0 = Some(sky);
        light_from_saved_layers(&mut chunk, &layers, true);
        world.install_light_column(chunk);
        if array_first {
            install_lower_neighbour(&mut world);
        }
        let chunk = &world.chunks[&(0, 0)];
        let storage = chunk.stored_light().unwrap();
        assert!(storage.presence(0)[0], "missing lower pad: {order}");
        let packed = storage.saved_arrays(0).unwrap()[0];
        assert_eq!(packed, array_first, "lower pad representation: {order}");
        assert!(
            (MIN_Y - 16..MIN_Y)
                .all(|y| (0..16).all(|z| (0..16).all(|x| chunk.light(x, y, z) >> 4 == 0))),
            "nonzero lower pad: {order}"
        );
        writeln!(report, "== {order}")?;
        writeln!(
            report,
            "storage {}",
            if packed { "packed" } else { "empty" }
        )?;

        let nbt = save::encode(chunk, assets.registry, &biomes, &items);
        let Some(Tag::List(sections)) = nbt.tag.get("sections") else {
            return Err("missing saved sections".into());
        };
        for section in sections {
            let Some(Tag::Byte(y)) = section.get("Y") else {
                return Err("missing saved section Y".into());
            };
            for key in ["SkyLight", "BlockLight"] {
                if let Some(tag) = section.get(key) {
                    let Tag::ByteArray(bytes) = tag else {
                        return Err("saved light is not a byte array".into());
                    };
                    write!(report, "save {y} {key} ")?;
                    append_hex(&mut report, bytes)?;
                }
            }
        }

        let bytes = encoder.body_in(chunk, &world)?;
        let mut packet = Reader::new(&bytes);
        assert_eq!(packet.varint()?, LEVEL_CHUNK_WITH_LIGHT);
        assert_eq!((packet.i32()?, packet.i32()?), (0, 0));
        ChunkData::read(&mut packet)?;
        report.push_str("packet ");
        append_hex(&mut report, packet.rest())?;
    }
    std::fs::write(output, report)?;
    Ok(())
}
