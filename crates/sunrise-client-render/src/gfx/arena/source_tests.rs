//! Original sidecar lifetime controls, executed explicitly on an offscreen adapter.
use super::*;
use crate::mesher::{TerrainSource, TerrainVertex};

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

#[test]
#[ignore = "daily"]
fn terrain_source_arena_gpu_grow_move_recycle_and_refusal() {
    let _gpu = crate::gfx::test_gpu::lock();
    let adapter = crate::gfx::test_gpu::adapter().expect("GPU control requires an adapter");
    eprintln!("TERRAIN_SOURCE_ARENA_ADAPTER {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let mut arena = Arena::new(&device, 24, GRAIN, true);
    arena.read_sources(true);
    arena.page_limit = GRAIN * 2;
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = super::super::upload::Uploads::new(&device);
    let first = arena.alloc(&device, &mut encoder, 4).unwrap();
    assert_eq!(arena.bytes(), GRAIN * 24);
    assert!(
        arena.sources(first.0).is_none(),
        "default geometry allocates no source buffer"
    );
    let geometry = [TerrainVertex::new([2.25, 3.75, 4.5], [0.25, 0.75], 7, 0xff123456, 0x8070); 4];
    uploads.buffer(
        &mut encoder,
        arena.buffer(first.0),
        first.1 * 24,
        bytemuck::cast_slice(&geometry),
    );
    let source = TerrainSource::new(71, [2, 3, 4], 13, false);
    assert_eq!(arena.vertex_bytes(), 24, "no sidecar yet");
    let written = arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        (first.0, first.1, 4),
        QuadSlots::vertices(4),
        &[source],
    );
    // The upload and the move budgets count the sidecar: 8 bytes a slot, 4 slots.
    assert_eq!(written, 32);
    assert_eq!(arena.vertex_bytes(), 24 + 8);
    let bytes_per_vertex = 24 + 8;
    assert_eq!(arena.bytes(), GRAIN * bytes_per_vertex);
    let second = arena.alloc(&device, &mut encoder, GRAIN).unwrap();
    assert_eq!(second, (first.0, GRAIN), "grew the first page");
    assert_eq!(arena.bytes(), 2 * GRAIN * bytes_per_vertex);
    let third = arena.alloc(&device, &mut encoder, GRAIN).unwrap();
    assert_ne!(third.0, first.0, "move crosses pages");
    assert_eq!(arena.bytes(), 4 * GRAIN * bytes_per_vertex);
    arena.copy(&mut encoder, first, third, 4);
    arena.release(first.0, first.1, 4);
    let reused = arena.alloc_existing(4).unwrap();
    assert_eq!(reused, first);
    arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        (reused.0, reused.1, 4),
        QuadSlots::vertices(4),
        &[],
    );
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    assert_eq!(
        read(
            &device,
            &queue,
            arena.sources(third.0).unwrap(),
            third.1 * 8,
            8
        ),
        bytemuck::bytes_of(&source)
    );
    assert_eq!(
        read(
            &device,
            &queue,
            arena.sources(first.0).unwrap(),
            first.1 * 8,
            8
        ),
        [0; 8]
    );
    assert_eq!(
        read(&device, &queue, arena.buffer(third.0), third.1 * 24, 96),
        bytemuck::cast_slice::<_, u8>(&geometry)
    );

    // A real requested binding limit refuses metadata/table allocation. The native
    // vertex allocation still fits; this is a controlled refusal, not a real OOM.
    let limits = wgpu::Limits {
        max_storage_buffer_binding_size: 256,
        ..Default::default()
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    }))
    .unwrap();
    let mut arena = Arena::translucent(&device, 24, GRAIN);
    arena.read_sources(true);
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = super::super::upload::Uploads::new(&device);
    let at = arena.alloc(&device, &mut encoder, 4).unwrap();
    uploads.buffer(
        &mut encoder,
        arena.buffer(at.0),
        at.1 * 24,
        bytemuck::cast_slice(&geometry),
    );
    arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        (at.0, at.1, 4),
        QuadSlots::vertices(4),
        &[source],
    );
    assert!(
        arena.sources(at.0).is_none(),
        "sidecar refused without losing vertices"
    );
    assert_eq!(arena.bytes(), GRAIN * 24);
    assert!(super::super::pack_post::block_ids(&device, &queue, &[0; 128], &[0, 1]).is_none());
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    assert_eq!(
        read(&device, &queue, arena.buffer(at.0), at.1 * 24, 96),
        bytemuck::cast_slice::<_, u8>(&geometry)
    );
}

/// A compact range's sources (five records, then padding and one quad kept as vertices) move
/// and recycle slot by slot at starts past slot 0; the arena keeps sources only while they
/// are read, and an arena nothing reads them from (the translucent one) never keeps any.
#[test]
#[ignore = "daily"]
fn terrain_sources_move_recycle_and_live_only_while_read() {
    let _gpu = crate::gfx::test_gpu::lock();
    let adapter = crate::gfx::test_gpu::adapter().expect("GPU control requires an adapter");
    eprintln!("TERRAIN_SOURCE_LIFE_ADAPTER {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let sources: Vec<_> = (0..5)
        .map(|q| TerrainSource::new(11 + 7 * q, [q as i32, 2, 3], q as u8, false))
        .collect();
    let slots = 12;
    let mut expected = vec![0u8; slots as usize * 8];
    expected[..40].copy_from_slice(bytemuck::cast_slice(&sources));

    let mut arena = Arena::new(&device, 24, GRAIN, true);
    arena.read_sources(true);
    arena.page_limit = GRAIN * 2;
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = super::super::upload::Uploads::new(&device);
    // A range past slot 0 of the first page, and its move to one past slot 0 of the second.
    let filler = arena.alloc(&device, &mut encoder, slots).unwrap();
    let from = arena.alloc(&device, &mut encoder, slots).unwrap();
    let other = arena.alloc(&device, &mut encoder, slots).unwrap();
    let to = arena.alloc(&device, &mut encoder, slots).unwrap();
    assert_eq!(
        [filler, from, other, to],
        [(0, 0), (0, GRAIN), (1, 0), (1, GRAIN)]
    );
    arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        (from.0, from.1, slots),
        QuadSlots::records(5),
        &sources,
    );
    arena.copy(&mut encoder, from, to, slots);
    // The range recycled for a mesh whose sources don't match its quads: every slot unknown.
    arena.release(from.0, from.1, slots);
    let reused = arena.alloc_existing(slots).unwrap();
    assert_eq!(reused, from);
    arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        (reused.0, reused.1, slots),
        QuadSlots::records(5),
        &sources[..3],
    );
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    let sidecar = |arena: &Arena, (page, start): (u32, u64)| {
        read(
            &device,
            &queue,
            arena.sources(page).unwrap(),
            start * 8,
            slots * 8,
        )
    };
    assert_eq!(sidecar(&arena, to), expected, "moved slot by slot");
    assert_eq!(
        sidecar(&arena, reused),
        vec![0; slots as usize * 8],
        "recycled"
    );

    // The pack is dropped: every sidecar goes, and its meshes still on their way add none.
    let released = arena.read_sources(false);
    assert_eq!(released.len(), 2);
    assert!(arena.sources(0).is_none() && arena.sources(1).is_none());
    assert_eq!(arena.bytes(), 4 * GRAIN * 24);
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = super::super::upload::Uploads::new(&device);
    arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        (to.0, to.1, slots),
        QuadSlots::records(5),
        &sources,
    );
    let fresh = arena.alloc(&device, &mut encoder, slots).unwrap();
    assert_eq!(fresh.0, 2, "a new page");
    assert!(arena.sources(0).is_none() && arena.sources(1).is_none() && arena.sources(2).is_none());
    // A pack loaded again keeps the sources of the meshes it brings.
    arena.read_sources(true);
    arena.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        (fresh.0, fresh.1, slots),
        QuadSlots::records(5),
        &sources,
    );
    assert!(arena.sources(2).is_some());
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    assert_eq!(sidecar(&arena, fresh), expected, "kept after the reload");

    // No draw reads the translucent arena's sources: it keeps none.
    let mut translucent = Arena::translucent(&device, 24, GRAIN);
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = super::super::upload::Uploads::new(&device);
    let at = translucent.alloc(&device, &mut encoder, 4).unwrap();
    translucent.write_sources(
        &device,
        &mut encoder,
        &mut uploads,
        (at.0, at.1, 4),
        QuadSlots::vertices(4),
        &sources[..1],
    );
    assert!(translucent.sources(at.0).is_none());
    assert_eq!(translucent.bytes(), GRAIN * 24);
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
}

/// Compaction's and a shrink's moves (`move_off_page`) count the source sidecar, which
/// `Arena::copy` moves with a range's vertices: once the pages have sidecars, 32 bytes a vertex
/// (24 of the vertex, 8 of its slot's source), so two meshes of 256 vertices are 16384 bytes and
/// a budget one byte short moves one of them, where 24 bytes a vertex would move both.
#[test]
#[ignore = "daily"]
fn source_sidecar_counts_in_the_move_budget() {
    use super::super::{Part, SectionGpu, move_off_page};
    let _gpu = crate::gfx::test_gpu::lock();
    let adapter = crate::gfx::test_gpu::adapter().expect("the source sidecar needs a GPU");
    eprintln!("TERRAIN_SOURCE_ARENA_ADAPTER {:?}", adapter.get_info());
    let (device, _queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let mut arena = Arena::translucent(&device, 24, 4 * GRAIN);
    arena.read_sources(true);
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = super::super::upload::Uploads::new(&device);
    let mesh = |at: (u32, u64)| SectionGpu {
        page: 0,
        start: 0,
        translucent_at: at,
        solid: 0,
        solid_slots: 0,
        packed: false,
        packed_far: false,
        sides: crate::mesher::Sides::unsorted(0),
        translucent: 64,
        sort: super::super::translucency::SortState::new(Vec::new()),
        sorted_indices: Some((7, 7)),
        flat_id: None,
        gpu_id: None,
        gpu_fading: false,
        fading: None,
        leaving: None,
        animated_layers: Box::new([]),
    };
    let mut meshes: Vec<SectionGpu> = (0..3)
        .map(|_| mesh(arena.alloc(&device, &mut encoder, GRAIN).unwrap()))
        .collect();
    assert_eq!(arena.vertex_bytes(), 24, "no sidecar yet");
    // A mesh with sources gives the pages their sidecars.
    let source = TerrainSource::new(71, [2, 3, 4], 13, false);
    let sources = vec![source; (GRAIN / 4) as usize];
    let first = meshes[0].translucent_at;
    let names = QuadSlots::vertices(GRAIN);
    let at = (first.0, first.1, GRAIN);
    arena.write_sources(&device, &mut encoder, &mut uploads, at, names, &sources);
    assert_eq!(arena.vertex_bytes(), 32);
    assert!(meshes.iter().all(|s| s.translucent_at.0 == 0));
    assert!(arena.open_page(&device, 4 * GRAIN));
    arena.close(0);
    let mut budget = 2 * GRAIN * 32 - 1;
    assert!(move_off_page(
        &mut arena,
        &mut encoder,
        None,
        None,
        None,
        meshes.iter_mut().map(|s| (s, None)),
        0,
        Part::Translucent,
        &mut budget,
    ));
    let moved = meshes.iter().filter(|s| s.translucent_at.0 == 1).count();
    assert_eq!(
        moved, 1,
        "one mesh of 8192 bytes, then 8191 left for the next"
    );
    assert_eq!(budget, GRAIN * 32 - 1);
}
