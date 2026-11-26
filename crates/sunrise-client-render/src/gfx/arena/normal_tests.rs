//! The normal stream beside an arena's vertices (`Arena::with_normals`): made, grown and moved
//! with its pages, and absent, the pages' bytes those of their vertices alone, without it.
use super::*;
use crate::gfx::terrain_normals::Entry;
use crate::mesher::{TerrainSource, TerrainVertex};

fn read(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer, offset: u64) -> Vec<u8> {
    let (tx, rx) = std::sync::mpsc::channel();
    wgpu::util::DownloadBuffer::read_buffer(
        device,
        queue,
        &buffer.slice(offset..offset + 4 * NORMAL_BYTES),
        move |result| tx.send(result.unwrap().to_vec()).unwrap(),
    );
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap()
}

/// A quad facing west (-x), U growing along +z: normal (-127, 0, 0), tangent (0, 0, 127), w 127.
fn west() -> [TerrainVertex; 4] {
    let uvs = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
    let corners = [[0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.]];
    std::array::from_fn(|i| TerrainVertex::new(corners[i], uvs[i], 0, u32::MAX, 0))
}

const WEST: [i8; 8] = [-127, 0, 0, 0, 0, 0, 127, 127];
/// The entry of a quad with no source: id -1, no valid flag.
const WEST_UNKNOWN: Entry = Entry {
    normal: WEST,
    source: [u32::MAX, 0],
};

#[test]
#[ignore = "daily"]
fn normal_stream_grows_and_moves_with_its_pages() {
    let _gpu = crate::gfx::test_gpu::lock();
    let adapter = crate::gfx::test_gpu::adapter().expect("the arena's normal stream needs a GPU");
    eprintln!("ARENA_NORMALS_ADAPTER {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();

    // Without the stream: the pages hold their vertices alone.
    let mut plain = Arena::new(&device, 24, GRAIN, false);
    let mut encoder = device.create_command_encoder(&Default::default());
    let at = plain.alloc(&device, &mut encoder, 4).unwrap();
    assert!(!plain.has_normals());
    assert!(plain.normals(at.0).is_none());
    assert!(plain.normals_binding(at.0).is_none());
    assert_eq!(plain.bytes(), GRAIN * 24);
    let mut uploads = super::super::upload::Uploads::new(&device);
    plain.write_normals(&mut encoder, &mut uploads, at, &west(), &[], &[]);
    assert_eq!(plain.bytes(), GRAIN * 24, "no stream is made for a write");

    let mut arena = Arena::new(&device, 24, GRAIN, false).with_normals(true);
    arena.page_limit = GRAIN * 2;
    let first = arena.alloc(&device, &mut encoder, 4).unwrap();
    assert_eq!(arena.bytes(), GRAIN * (24 + 16));
    arena.write_normals(&mut encoder, &mut uploads, first, &west(), &[], &[]);
    // The page is full: the next range grows it, in the same encoder.
    let grown = arena.alloc(&device, &mut encoder, GRAIN).unwrap();
    assert_eq!(grown, (first.0, GRAIN), "grew the first page");
    assert_eq!(arena.bytes(), 2 * GRAIN * (24 + 16));
    // A third range starts a page of its own, and the quad moves there.
    let other = arena.alloc(&device, &mut encoder, GRAIN).unwrap();
    assert_ne!(other.0, first.0);
    assert_eq!(arena.bytes(), 4 * GRAIN * (24 + 16));
    arena.copy(&mut encoder, first, other, 4);
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    let expected: Vec<u8> = bytemuck::cast_slice(&[WEST_UNKNOWN; 4]).to_vec();
    assert_eq!(
        read(
            &device,
            &queue,
            arena.normals(first.0).unwrap(),
            first.1 * NORMAL_BYTES
        ),
        expected,
        "written before the grow, kept in the grown page"
    );
    assert_eq!(
        read(
            &device,
            &queue,
            arena.normals(other.0).unwrap(),
            other.1 * NORMAL_BYTES
        ),
        expected,
        "moved with its vertices"
    );
    // Draws bind the stream as far as the vertices: the grown page's two ranges.
    let bound = arena.normals_binding(first.0).unwrap();
    assert_eq!(bound.size(), 2 * GRAIN * NORMAL_BYTES);
    // A page opened for a shrink's moves has its stream too.
    let opened = arena.pages.len() as u32;
    assert!(arena.open_page(&device, GRAIN));
    assert!(
        arena.normals(opened).is_some(),
        "an opened page has a stream"
    );
    assert_eq!(arena.bytes(), 5 * GRAIN * (24 + 16));
    // The opened page holds no range: trimmed, its vertices and its stream go back together.
    assert_eq!(arena.trim().len(), 2, "the empty page's buffer and stream");
    // Two pages left, each a buffer and a stream.
    assert_eq!(arena.clear().len(), 4);
    assert_eq!(arena.bytes(), 0);

    // A mesh longer than a chunk of the stream (`terrain_normals::CHUNK`, 1024 vertices): 256
    // west quads, then an up one at vertices 1024 to 1027, whose entries land past the chunk,
    // each quad with a source of its own state: the last of the first chunk state 255, the one
    // past it state 256, whose ids are 2255 and 2256.
    let mut long = Arena::new(&device, 24, GRAIN, false).with_normals(true);
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = super::super::upload::Uploads::new(&device);
    let at = long.alloc(&device, &mut encoder, 1028).unwrap();
    let mut vertices: Vec<TerrainVertex> = (0..256).flat_map(|_| west()).collect();
    let uvs = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
    let up = [[0., 1., 1.], [1., 1., 1.], [1., 1., 0.], [0., 1., 0.]];
    vertices.extend((0..4).map(|i| TerrainVertex::new(up[i], uvs[i], 0, u32::MAX, 0)));
    let sources: Vec<_> = (0..257)
        .map(|state| TerrainSource::new(state, [1, 2, 3], 0, false))
        .collect();
    let ids: Vec<i32> = (0..257).flat_map(|state| [2000 + state, -1]).collect();
    long.write_normals(&mut encoder, &mut uploads, at, &vertices, &sources, &ids);
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    let stream = long.normals(at.0).unwrap();
    // Cell (1, 2, 3) and the valid flag.
    let word = 1 | 2 << 4 | 3 << 8 | 1 << 13;
    let west_entry = Entry {
        normal: WEST,
        source: [2255, word],
    };
    let up = Entry {
        normal: [0, 127, 0, 0, 127, 0, 0, 127],
        source: [2256, word],
    };
    assert_eq!(
        read(&device, &queue, stream, (at.1 + 1020) * NORMAL_BYTES),
        bytemuck::cast_slice::<Entry, u8>(&[west_entry; 4]).to_vec(),
        "the last quad of the first chunk"
    );
    assert_eq!(
        read(&device, &queue, stream, (at.1 + 1024) * NORMAL_BYTES),
        bytemuck::cast_slice::<Entry, u8>(&[up; 4]).to_vec(),
        "the quad past the first chunk"
    );

    // Sources that are not one a quad (one for two quads, as a mesh that recorded none for some
    // of its quads would have): neither quad takes one, as `write_sources` keeps none then.
    let mut encoder = device.create_command_encoder(&Default::default());
    let mut uploads = super::super::upload::Uploads::new(&device);
    let two = long.alloc(&device, &mut encoder, 8).unwrap();
    let pair: Vec<TerrainVertex> = (0..2).flat_map(|_| west()).collect();
    long.write_normals(&mut encoder, &mut uploads, two, &pair, &sources[..1], &ids);
    uploads.finish(&encoder);
    queue.submit([encoder.finish()]);
    let stream = long.normals(two.0).unwrap();
    for quad in 0..2 {
        assert_eq!(
            read(&device, &queue, stream, (two.1 + 4 * quad) * NORMAL_BYTES),
            bytemuck::cast_slice::<Entry, u8>(&[WEST_UNKNOWN; 4]).to_vec(),
            "quad {quad} of a mesh whose sources are not one a quad"
        );
    }
}

/// The budgets that keep a frame's copies small count the normal stream: compaction's and a
/// shrink's moves (`move_off_page`), which copy a range's entries with its vertices, and the
/// emptying a fragmented arena starts at (`sparsest`), whose free ranges hold entries too. With
/// 40 bytes a vertex (24 of the vertex, 16 of its entry), two meshes of 256 vertices are 20480
/// bytes: a budget one byte short moves one of them, where 24 bytes a vertex would move both.
#[test]
#[ignore = "daily"]
fn normal_stream_counts_in_the_move_budgets() {
    use super::super::{Part, SectionGpu, move_off_page};
    let _gpu = crate::gfx::test_gpu::lock();
    let adapter = crate::gfx::test_gpu::adapter().expect("the arena's normal stream needs a GPU");
    eprintln!("ARENA_NORMALS_ADAPTER {:?}", adapter.get_info());
    let (device, _queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let mut arena = Arena::translucent(&device, 24, 4 * GRAIN).with_normals(true);
    assert_eq!(arena.vertex_bytes(), 40);
    let mut encoder = device.create_command_encoder(&Default::default());
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
    assert!(meshes.iter().all(|s| s.translucent_at.0 == 0));
    assert!(arena.open_page(&device, 4 * GRAIN));
    arena.close(0);
    let mut budget = 2 * GRAIN * 40 - 1;
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
        "one mesh of 10240 bytes, then 10239 left for the next"
    );
    assert_eq!(budget, GRAIN * 40 - 1);

    // A fragmented arena: one range of 256 vertices in use on page 0 and two on page 1, of
    // four each; the five free ranges hold 1280 vertices, 51200 bytes with their entries.
    let mut arena = Arena::new(&device, 24, GRAIN, false).with_normals(true);
    arena.page_limit = 4 * GRAIN;
    let ranges: Vec<(u32, u64)> = (0..8)
        .map(|_| arena.alloc(&device, &mut encoder, GRAIN).unwrap())
        .collect();
    let first = ranges[0].0;
    for (i, &(page, start)) in ranges.iter().enumerate() {
        if ![0, 6, 7].contains(&i) {
            arena.release(page, start, GRAIN);
        }
    }
    assert_eq!(arena.sparsest_from(5 * GRAIN * 40), Some(first));
    assert_eq!(arena.sparsest_from(5 * GRAIN * 40 + 1), None);
}

/// A page shrinks (`oversized`) from `SHRINK_MIN_BYTES` (64 MiB) of it, its normal stream
/// counted: 1677824 vertices (6554 grains) of 40 bytes are just over 64 MiB with the stream (a
/// grain fewer, just under), 38.4 MiB without. Holding one range, it moves into a page of the
/// first page's size (2^18 vertices).
#[test]
#[ignore = "daily"]
fn normal_stream_counts_toward_a_shrink() {
    let _gpu = crate::gfx::test_gpu::lock();
    let adapter = crate::gfx::test_gpu::adapter().expect("the arena's normal stream needs a GPU");
    let (device, _queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    // `oversized` looks at the counts only: a small buffer stands in for the page.
    let buffer = try_create(&device, "Stand-in page", 4, wgpu::BufferUsages::VERTEX).unwrap();
    let capacity = 1_677_824;
    let page = || Page {
        buffer: buffer.clone(),
        sources: None,
        normals: None,
        capacity,
        free: FreeList(BTreeMap::from([(GRAIN, capacity - GRAIN)])),
        host: false,
    };
    let mut with = Arena::translucent(&device, 24, 1 << 18).with_normals(true);
    with.page_limit = LARGE_PAGE_BYTES / 24 / GRAIN * GRAIN;
    with.pages = vec![Some(page())];
    assert_eq!(with.oversized(), Some((0, 1 << 18)));
    let mut without = Arena::translucent(&device, 24, 1 << 18);
    without.page_limit = with.page_limit;
    without.pages = vec![Some(page())];
    assert_eq!(without.oversized(), None);
}
