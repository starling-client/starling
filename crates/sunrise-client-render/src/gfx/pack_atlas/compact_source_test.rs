//! A shader pack's block ids (`mc_Entity`) and owning cells (`at_midBlock`) for terrain in the
//! compact layout (`mesher::packed`) and in the wide one: the mesher's sources reach the arena
//! by the slot that names each quad, and flat draws read them from there.

use super::midpoint_test::{Page, sample_page};
use crate::gfx::arena::{Arena, GRAIN};
use crate::gfx::section_upload::solid_sources;
use crate::mesher::source_tests::{SHAPED_BLOCKS, shaped_mesh};
use crate::mesher::{SectionMesh, TerrainSource, TerrainVertex, packed};
use sunrise_render_shaderpack::pack_atlas::PackAtlas;

/// The block of `SHAPED_BLOCKS` whose cell holds all four corners of `quad`: (state, cell).
fn owner(quad: &[TerrainVertex]) -> (u32, [i32; 3]) {
    let inside = |cell: [i32; 3]| {
        quad.iter()
            .all(|v| (0..3).all(|a| (cell[a] as f32..=cell[a] as f32 + 1.0).contains(&v.pos[a])))
    };
    let mut owners = SHAPED_BLOCKS.iter().filter(|(_, cell)| inside(*cell));
    let found = *owners.next().expect("a face in a block's cell");
    assert!(owners.next().is_none(), "blocks are apart");
    found
}

/// The ids `block.properties` gives the fixture's states in these tests, distinct and nonzero:
/// cube 10, stair 20, door 30, path 40 (fluids -1). Pairs per state (model, fluid).
const IDS: [i32; 10] = [-1, -1, 10, -1, 20, -1, 30, -1, 40, -1];

/// The fixture's quads: (state, cell) of each, by the corners the mesher made.
fn owners(mesh: &SectionMesh) -> Vec<(u32, [i32; 3])> {
    mesh.solid.chunks(4).map(owner).collect()
}

#[test]
fn each_quads_source_lies_at_the_slot_naming_it_in_either_layout() {
    for compact in [true, false] {
        let mesh = shaped_mesh(compact);
        let quads = mesh.solid.len() / 4;
        let slots = if compact {
            let records = packed::packed_quads(&mesh.packed, quads);
            assert!(
                records > 0 && records < quads,
                "on-grid faces get records, off-grid ones stay vertices: {records} of {quads}"
            );
            mesh.packed.len()
        } else {
            assert!(mesh.packed.is_empty());
            mesh.solid.len()
        };
        let mut laid_out = Vec::new();
        let (names, sources) = solid_sources(&mesh, compact);
        assert!(
            names.lay_out(sources, slots as u64, &mut laid_out),
            "compact={compact}: the mesh's sources are written"
        );
        let mut expected = vec![TerrainSource::default(); laid_out.len()];
        for (q, (state, cell)) in owners(&mesh).into_iter().enumerate() {
            let slot = if compact { q } else { 4 * q };
            expected[slot] = TerrainSource::new(state, cell, 0, false);
        }
        assert_eq!(laid_out, expected, "compact={compact}");
    }
}

/// Reads `size` bytes at `offset` of `buffer`.
fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    offset: u64,
    size: u64,
) -> Vec<u8> {
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, offset, &out, 0, size);
    queue.submit([encoder.finish()]);
    out.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    out.slice(..).get_mapped_range().unwrap().to_vec()
}

/// The issue's case: on-grid faces (records) and off-grid ones (kept as vertices) of a cube, a
/// stair, a door and a path, uploaded as a section's solid range is, each read its own block's
/// id and cell in a pack's terrain program; and so do the same blocks in the wide layout, in the
/// same page.
#[test]
#[ignore = "daily"]
fn a_pack_reads_each_faces_own_block_from_compact_and_wide_terrain() {
    let _gpu = crate::gfx::test_gpu::lock();
    let adapter = crate::gfx::test_gpu::adapter().expect("GPU test requires an adapter");
    eprintln!("COMPACT_SOURCE_ADAPTER {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    let atlas = PackAtlas::new(&device, &mut encoder, &[texture], 1, 1, 16).unwrap();
    queue.submit([encoder.finish()]);

    // The wide section first, so the compact one's slots start past it, as in a filled page.
    let meshes = [shaped_mesh(false), shaped_mesh(true)];
    let mut arena = Arena::new(&device, 24, GRAIN, true);
    // A pack draws terrain from it (`Renderer::keep_pack_sources`).
    arena.read_sources(true);
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = crate::gfx::upload::Uploads::new(&device);
    let mut ranges = Vec::new();
    for (mesh, compact) in meshes.iter().zip([false, true]) {
        let bytes: &[u8] = if compact {
            bytemuck::cast_slice(&mesh.packed)
        } else {
            bytemuck::cast_slice(&mesh.solid)
        };
        let slots = bytes.len() as u64 / 24;
        let (page, start) = arena.alloc(&device, &mut encoder, slots).unwrap();
        uploads.buffer(&mut encoder, arena.buffer(page), start * 24, bytes);
        let (names, sources) = solid_sources(mesh, compact);
        arena.write_sources(
            &device,
            &mut encoder,
            &mut uploads,
            (page, start, slots),
            names,
            sources,
        );
        ranges.push((page, start));
    }
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    assert_eq!(ranges[0].0, ranges[1].0, "one page holds both");
    assert!(ranges[1].1 > 0, "the compact range starts past slot 0");
    let page = ranges[0].0;
    let vertices = arena.buffer(page);
    let slots = read(&device, &queue, vertices, 0, vertices.size());
    let sources = arena
        .sources(page)
        .expect("a sidecar for the pack's sources");
    let sources = read(&device, &queue, sources, 0, sources.size());

    let mut failures = Vec::new();
    for ((mesh, compact), (_, start)) in meshes.iter().zip([false, true]).zip(&ranges) {
        let quads = mesh.solid.len() as u32 / 4;
        let start = *start as u32;
        // `SectionGpu::solid_base`: a compact draw names a record's corners past `PACKED_BASE`.
        let base = if compact {
            packed::PACKED_BASE + 4 * start
        } else {
            start
        };
        let drawn = sample_page(
            &device,
            &queue,
            &atlas,
            Page {
                slots: &slots,
                sources: &sources,
                ids: &IDS,
            },
            base..base + 4 * quads,
            1.0,
            [true, false],
            "vec4(mc_Entity.x, at_midBlock.xyz)",
        );
        for (q, (state, cell)) in owners(mesh).into_iter().enumerate() {
            for corner in 0..4 {
                let vertex = &mesh.solid[4 * q + corner];
                let mid = |a: usize| (cell[a] as f32 + 0.5 - vertex.pos[a]) * 64.0;
                let expected = [IDS[2 * state as usize] as f32, mid(0), mid(1), mid(2)];
                let got = drawn[4 * q + corner];
                if got != expected {
                    failures.push(format!(
                        "compact={compact} quad {q} corner {corner}: {got:?} != {expected:?}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
