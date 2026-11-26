//! Readbacks of retained storage buffers behind [`mod_api::ReadbackTicket`]: vanilla's
//! `CommandEncoder.copyToBuffer(source, target)` into a `USAGE_MAP_READ` buffer, a fence checked
//! without waiting, then `GpuBuffer.map(read)`.
//!
//! A request made inside a frame-graph compute node's `record` callback allocates the staging
//! buffer at once and notes its place among the node's dispatches. The frame graph encodes the
//! copy at that place when it runs the node (so it sees the writes of the dispatches recorded
//! before it, as an encoder command does), and [`ModReadbacks::submitted`] starts the
//! `map_async` of every copy that was encoded right after the frame's submission. The frame's
//! one `PollType::Poll` takes the completions in; nothing here ever waits on the device.

use mod_api::__private::game::Fields;
use mod_api::{BufferError, ReadbackTicket};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

/// The map has not finished.
const PENDING: u8 = 0;
/// The map finished and the bytes can be read.
const READY: u8 = 1;
/// The map failed (the device was lost, or the buffer went away).
const FAILED: u8 = 2;

/// One copy a compute node holds, encoded by the frame graph after the first `after` of the
/// node's recorded dispatches.
#[derive(Clone, Debug)]
pub struct RecordedReadback {
    /// How many of the node's dispatches were recorded before the request.
    pub after: usize,
    pub source: wgpu::Buffer,
    pub source_offset: u64,
    pub staging: wgpu::Buffer,
    pub len: u64,
    /// Set when the frame graph encoded the copy.
    pub copied: Arc<AtomicBool>,
}

impl RecordedReadback {
    /// Encode the copy on `encoder`.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.copy_buffer_to_buffer(&self.source, self.source_offset, &self.staging, 0, self.len);
        self.copied.store(true, Ordering::Release);
    }
}

enum Phase {
    /// Requested in this frame; the copy is encoded when the node runs.
    Requested,
    /// The copy was submitted and its map started.
    Mapping,
    /// The frame did not run the node: the copy never happened.
    Lost,
}

struct Ticket {
    owner: Option<&'static str>,
    staging: wgpu::Buffer,
    copied: Arc<AtomicBool>,
    mapped: Arc<AtomicU8>,
    phase: Phase,
}

#[derive(Default)]
struct Recording {
    dispatches: usize,
    requests: Vec<RecordedReadback>,
}

/// The pending readbacks of one renderer, shared with the mod hooks that run while the renderer
/// is out of the game.
#[derive(Default)]
pub struct ModReadbacks {
    next_id: u64,
    tickets: HashMap<u64, Ticket>,
    recording: Option<Recording>,
}

/// The compute node being recorded: readback requests are accepted while it lives. Dropping it
/// without [`Self::finish`] (a panicking listener) closes the recording.
pub struct ReadbackRecording {
    readbacks: Arc<Mutex<ModReadbacks>>,
}

impl ReadbackRecording {
    /// Open the recording of a compute node on `readbacks`.
    pub fn begin(readbacks: &Arc<Mutex<ModReadbacks>>) -> Self {
        readbacks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recording = Some(Recording::default());
        Self {
            readbacks: readbacks.clone(),
        }
    }

    /// Close the recording and take the copies requested in it, in request order.
    pub fn finish(self) -> Vec<RecordedReadback> {
        self.readbacks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recording
            .take()
            .map_or_else(Vec::new, |recording| recording.requests)
    }
}

impl Drop for ReadbackRecording {
    fn drop(&mut self) {
        self.readbacks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recording = None;
    }
}

impl ModReadbacks {
    /// Note that the node being recorded took another dispatch.
    pub fn note_dispatch(&mut self) {
        if let Some(recording) = &mut self.recording {
            recording.dispatches += 1;
        }
    }

    /// Allocate the staging buffer of a copy of `len` bytes from `offset` of `source`, to be
    /// encoded at this point of the node being recorded. Everything is checked before the device
    /// is asked; the allocation sits in error scopes, so a device refusal is an `Err`.
    pub fn request(
        &mut self,
        device: &wgpu::Device,
        owner: Option<&'static str>,
        source: wgpu::Buffer,
        offset: u64,
        len: u64,
    ) -> Result<ReadbackTicket, BufferError> {
        if len == 0 {
            return Err(BufferError::Empty);
        }
        if !offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
            || !len.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
        {
            return Err(BufferError::InvalidAlignment);
        }
        if !source.usage().contains(wgpu::BufferUsages::COPY_SRC) {
            return Err(BufferError::NotReadable);
        }
        let end = offset.checked_add(len).ok_or(BufferError::OutOfRange)?;
        if end > source.size() {
            return Err(BufferError::OutOfRange);
        }
        let Some(recording) = self.recording.as_mut() else {
            return Err(BufferError::NotInFrame);
        };
        let id = self
            .next_id
            .checked_add(1)
            .ok_or(BufferError::Unavailable)?;
        let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Mod readback staging"),
            size: len,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let validation = pollster::block_on(validation.pop());
        let out_of_memory = pollster::block_on(out_of_memory.pop());
        if let Some(error) = validation {
            log::warn!("Sunrise: a mod readback buffer was refused by the device: {error}");
            return Err(BufferError::ExceedsDeviceLimit);
        }
        if let Some(error) = out_of_memory {
            log::warn!("Sunrise: the device has no memory for a mod readback buffer: {error}");
            return Err(BufferError::ExceedsDeviceLimit);
        }
        self.next_id = id;
        let copied = Arc::new(AtomicBool::new(false));
        recording.requests.push(RecordedReadback {
            after: recording.dispatches,
            source,
            source_offset: offset,
            staging: staging.clone(),
            len,
            copied: copied.clone(),
        });
        self.tickets.insert(
            id,
            Ticket {
                owner,
                staging,
                copied,
                mapped: Arc::new(AtomicU8::new(PENDING)),
                phase: Phase::Requested,
            },
        );
        Ok(mod_api::__private::built::readback_ticket(id))
    }

    /// Start the maps of the copies the frame encoded, once it is submitted (the returned work
    /// runs right after the submission), and give up the copies it did not.
    pub fn submitted(&mut self) -> Option<super::submit::AfterSubmit> {
        let mut maps = Vec::new();
        for ticket in self.tickets.values_mut() {
            if !matches!(ticket.phase, Phase::Requested) {
                continue;
            }
            if ticket.copied.load(Ordering::Acquire) {
                ticket.phase = Phase::Mapping;
                maps.push((ticket.staging.clone(), ticket.mapped.clone()));
            } else {
                ticket.phase = Phase::Lost;
            }
        }
        if maps.is_empty() {
            return None;
        }
        Some(Box::new(move || {
            for (staging, mapped) in maps {
                staging.map_async(wgpu::MapMode::Read, .., move |result| {
                    mapped.store(
                        if result.is_ok() { READY } else { FAILED },
                        Ordering::Release,
                    );
                });
            }
        }))
    }

    /// The bytes of `ticket` if its copy has landed, `None` while it is in flight. Never waits.
    pub fn poll(
        &mut self,
        owner: Option<&'static str>,
        ticket: ReadbackTicket,
    ) -> Result<Option<Vec<u8>>, BufferError> {
        let id = ticket.fields().generation;
        let entry = self.tickets.get(&id).ok_or(BufferError::Unavailable)?;
        if entry.owner != owner {
            return Err(BufferError::NotOwned);
        }
        let bytes = match entry.phase {
            Phase::Requested => return Ok(None),
            Phase::Lost => None,
            Phase::Mapping => match entry.mapped.load(Ordering::Acquire) {
                READY => {
                    let bytes = entry
                        .staging
                        .get_mapped_range(..)
                        .ok()
                        .map(|view| view.to_vec());
                    entry.staging.unmap();
                    bytes
                }
                FAILED => None,
                _ => return Ok(None),
            },
        };
        self.tickets.remove(&id);
        bytes.map(Some).ok_or(BufferError::Unavailable)
    }

    /// Drop `ticket` and free its staging buffer.
    pub fn release(
        &mut self,
        owner: Option<&'static str>,
        ticket: ReadbackTicket,
    ) -> Result<(), BufferError> {
        let id = ticket.fields().generation;
        let entry = self.tickets.get(&id).ok_or(BufferError::Unavailable)?;
        if entry.owner != owner {
            return Err(BufferError::NotOwned);
        }
        self.tickets.remove(&id);
        Ok(())
    }

    /// Drop every ticket owned by a mod in `unloaded`.
    pub fn retire_unloaded(&mut self, unloaded: &BTreeSet<String>) {
        self.tickets
            .retain(|_, t| !t.owner.is_some_and(|id| unloaded.contains(id)));
    }
}

impl super::mod_gpu::ModGpuResources {
    /// Open the recording of a frame-graph compute node: readback requests are accepted until
    /// the returned recording is finished or dropped.
    pub fn begin_readback_recording(&self) -> ReadbackRecording {
        ReadbackRecording::begin(&self.readbacks)
    }

    /// Note that the compute node being recorded took another dispatch.
    pub fn note_compute_dispatch(&self) {
        self.readbacks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .note_dispatch();
    }

    /// Request a readback of `len` bytes from `offset` of a retained storage buffer.
    pub fn storage_buffer_request_readback(
        &self,
        owner: Option<&'static str>,
        buffer: mod_api::StorageBufferRef,
        offset: u64,
        len: u64,
    ) -> Result<ReadbackTicket, BufferError> {
        let source = self.resolve_mod_storage_buffer(owner, buffer)?;
        self.readbacks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .request(&self.device, owner, source, offset, len)
    }

    /// The bytes of a readback, `None` while it is in flight.
    pub fn readback_poll(
        &self,
        owner: Option<&'static str>,
        ticket: ReadbackTicket,
    ) -> Result<Option<Vec<u8>>, BufferError> {
        self.readbacks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .poll(owner, ticket)
    }

    /// Drop a readback that is no longer wanted.
    pub fn readback_release(
        &self,
        owner: Option<&'static str>,
        ticket: ReadbackTicket,
    ) -> Result<(), BufferError> {
        self.readbacks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .release(owner, ticket)
    }

    /// Free the pending readbacks of mods that unloaded.
    pub fn retire_unloaded_mod_readbacks(&self, unloaded: &BTreeSet<String>) {
        self.readbacks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retire_unloaded(unloaded);
    }

    /// The work that starts the maps of the frame's copies right after its submission.
    pub(super) fn readbacks_submitted(&self) -> Option<super::submit::AfterSubmit> {
        self.readbacks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .submitted()
    }
}

#[cfg(test)]
mod tests {
    use super::super::mod_compute::{DispatchKind, RecordedDispatch};
    use super::super::mod_frame_graph::{
        FrameGraphRunner, FrameGraphSlot, RecordedFrameGraph, RecordedFramePass,
        TransientTargetPool,
    };
    use super::super::mod_pipelines::ResolvedSet1Entry;
    use super::super::mod_render_targets::ModRenderTargets;
    use super::super::mod_storage::ModStorageBuffers;
    use super::super::world_primitives::WorldPrimitives;
    use super::*;
    use mod_api::{
        BindEntry, BindGroupLayout, BindingKind, ComputePassDescriptor, ComputePipeline,
        Identifier, StorageBufferDescriptor,
    };

    const FILL: &str = "#version 450\nlayout(local_size_x = 8) in;\nlayout(std430) buffer Out { uint values[]; };\nvoid main() { values[gl_GlobalInvocationID.x] = gl_GlobalInvocationID.x * 3u + 1u; }\n";
    const LATER: &str = "#version 450\nlayout(local_size_x = 8) in;\nlayout(std430) buffer Out { uint values[]; };\nvoid main() { values[gl_GlobalInvocationID.x] = 77u; }\n";

    fn pipeline(name: &str) -> ComputePipeline {
        let layout = BindGroupLayout::builder()
            .entry(BindEntry::new(
                "Out",
                BindingKind::StorageBuffer {
                    read_only: false,
                    min_size: 4,
                },
            ))
            .build()
            .unwrap();
        ComputePipeline::builder(
            Identifier::parse(name).unwrap(),
            Identifier::parse(name).unwrap(),
        )
        .bind_group_layout(layout)
        .build()
    }

    #[test]
    #[ignore = "daily"]
    fn a_readback_requested_in_a_compute_node_returns_what_the_dispatches_before_it_wrote() {
        let _gpu = crate::gfx::test_gpu::lock();
        let Some((_, device, queue)) = crate::gfx::test_gpu::optional_device() else {
            return;
        };
        let set = crate::mod_compute::ComputeSet::build(
            &[pipeline("t:fill"), pipeline("t:later")],
            1,
            |file| {
                Ok(match file {
                    "assets/t/shaders/compute/fill.csh" => Some(FILL.to_owned()),
                    "assets/t/shaders/compute/later.csh" => Some(LATER.to_owned()),
                    _ => None,
                })
            },
        );
        let mut primitives =
            WorldPrimitives::new(&device, (wgpu::TextureFormat::Rgba8Unorm, false));
        primitives.pipelines.compute.build(&device, &set);

        let mut store = ModStorageBuffers::default();
        let plain = store
            .create(&device, None, &StorageBufferDescriptor::new(32))
            .unwrap();
        let handle = store
            .create(&device, None, &StorageBufferDescriptor::new(32).copy_src())
            .unwrap();
        let buffer = store.resolve_buffer(None, handle).unwrap();
        let plain_buffer = store.resolve_buffer(None, plain).unwrap();
        let dispatch = |slot| RecordedDispatch {
            slot,
            entries: vec![ResolvedSet1Entry::StorageBuffer {
                id: 1,
                buffer: buffer.clone(),
                offset: 0,
                size: 32,
            }],
            inline: Vec::new(),
            texel: Vec::new(),
            kind: DispatchKind::Direct([1, 1, 1]),
        };

        // The node: one dispatch, the request, a second dispatch that overwrites the buffer.
        let readbacks = Arc::new(Mutex::new(ModReadbacks::default()));
        assert_eq!(
            readbacks
                .lock()
                .unwrap()
                .request(&device, None, buffer.clone(), 0, 32)
                .err(),
            Some(BufferError::NotInFrame)
        );
        let recording = ReadbackRecording::begin(&readbacks);
        let mut pass = RecordedFramePass::from_compute_descriptor(
            ComputePassDescriptor::new("readback node"),
            None,
        );
        let dispatches = pass.dispatches.as_mut().unwrap();
        dispatches.push(dispatch(0));
        readbacks.lock().unwrap().note_dispatch();
        {
            let mut table = readbacks.lock().unwrap();
            assert_eq!(
                table
                    .request(&device, None, plain_buffer.clone(), 0, 32)
                    .err(),
                Some(BufferError::NotReadable)
            );
            assert_eq!(
                table.request(&device, None, buffer.clone(), 2, 8).err(),
                Some(BufferError::InvalidAlignment)
            );
            assert_eq!(
                table.request(&device, None, buffer.clone(), 28, 8).err(),
                Some(BufferError::OutOfRange)
            );
            assert_eq!(
                table.request(&device, None, buffer.clone(), 0, 0).err(),
                Some(BufferError::Empty)
            );
        }
        let ticket = readbacks
            .lock()
            .unwrap()
            .request(&device, None, buffer.clone(), 0, 32)
            .unwrap();
        let dispatches = pass.dispatches.as_mut().unwrap();
        dispatches.push(dispatch(1));
        readbacks.lock().unwrap().note_dispatch();
        pass.readbacks = recording.finish();
        assert_eq!(pass.readbacks.len(), 1);
        assert_eq!(pass.readbacks[0].after, 1);

        // A request in the same node that no frame runs is given up at the submission.
        let recording = ReadbackRecording::begin(&readbacks);
        let unrun = readbacks
            .lock()
            .unwrap()
            .request(&device, None, buffer.clone(), 0, 4)
            .unwrap();
        drop(recording);

        let mut graph = RecordedFrameGraph::default();
        graph.push_pass(pass);
        let mut warned = std::collections::HashSet::new();
        let mut runner = FrameGraphRunner::prepare(graph, false, &mut warned).expect("graph");
        let targets = ModRenderTargets::default();
        let mut pool = TransientTargetPool::default();
        let mut encoder = device.create_command_encoder(&Default::default());
        runner.execute_slot(
            FrameGraphSlot::BeforeWorld,
            &device,
            &queue,
            &mut encoder,
            &mut primitives,
            &targets,
            &mut pool,
        );
        runner.finish(&mut pool);
        let after = readbacks.lock().unwrap().submitted();
        queue.submit([encoder.finish()]);
        let mut table = readbacks.lock().unwrap();
        // The copy has not been mapped yet.
        assert_eq!(table.poll(None, ticket), Ok(None));
        after.expect("a map to start")();
        assert_eq!(
            table.poll(Some("other"), ticket),
            Err(BufferError::NotOwned)
        );
        assert_eq!(table.poll(None, unrun), Err(BufferError::Unavailable));

        // The device finishes the copy in its own time: only non-blocking polls, over the next
        // frames, until the bytes arrive.
        let mut bytes = None;
        for _ in 0..1000 {
            let _ = device.poll(wgpu::PollType::Poll);
            if let Some(found) = table.poll(None, ticket).unwrap() {
                bytes = Some(found);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let bytes = bytes.expect("the readback to land");
        // Eight words of the first dispatch's pattern: the later dispatch is not seen.
        let values: &[u32] = bytemuck::cast_slice(&bytes);
        assert_eq!(values, [1, 4, 7, 10, 13, 16, 19, 22]);
        // A spent ticket is unknown.
        assert_eq!(table.poll(None, ticket), Err(BufferError::Unavailable));
        assert_eq!(table.release(None, ticket), Err(BufferError::Unavailable));
    }
}
