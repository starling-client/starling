//! A client mod's retained GPU storage buffers (`ClientRenderClientHost::create_storage_buffer`,
//! `StorageBufferRef::write_range`, `size` and `release`): vanilla's `GpuBuffer` written through
//! `CommandEncoder.writeToBuffer(GpuBufferSlice, ByteBuffer)` and bound by range
//! (`GpuBuffer.slice`, `GpuBufferSlice`), and read back without waiting
//! (`StorageBufferRef::request_readback`, `ReadbackTicket`): vanilla's
//! `CommandEncoder.copyToBuffer(source, target)` into a `USAGE_MAP_READ` buffer, then
//! `GpuBuffer.map(read)` once its fence has signalled.

use super::render_pipeline::BufferError;

/// A live client context that can operate on retained GPU [`StorageBufferRef`] handles.
pub trait StorageBufferContext {
    /// The scoped engine access used by [`StorageBufferRef`] methods.
    // hidden: internal trait method opening the retained-buffer backend for StorageBufferRef.
    #[doc(hidden)]
    fn storage_buffer_backend(&self) -> Option<StorageBufferBackendRef<'_>> {
        None
    }
}

/// Opaque access to retained storage-buffer operations, not to the world or other client services.
// hidden: only the handle can open the storage-buffer backend.
// exhaustive: private field; only constructed inside this crate.
// no wire: scoped backend borrow.
#[doc(hidden)]
pub struct StorageBufferBackendRef<'a>(&'a dyn crate::prelude::ClientRenderClientBackend);

impl<H: crate::ClientHost + ?Sized> StorageBufferContext for H {
    fn storage_buffer_backend(&self) -> Option<StorageBufferBackendRef<'_>> {
        self.backend()
            .map(|backend| StorageBufferBackendRef(backend.open()))
    }
}

/// Generational handle to a retained GPU storage buffer, created via
/// [`crate::prelude::ClientRenderClientHost::create_storage_buffer`].
///
/// Handles are owner-checked. The buffer outlives a resource reload and a world close, and is
/// freed by [`Self::release`] or when its mod unloads.
// no wire: process-local generational handle to a retained GPU storage buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the engine creates handles.
pub struct StorageBufferRef(pub(crate) u64, pub(crate) u32);

impl StorageBufferRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished storage buffer generation");
        }
        Ok(Self(generation, slot))
    }

    /// Queue `data` for `offset` of this buffer (`CommandEncoder.writeToBuffer` on a
    /// `GpuBufferSlice`). `offset` and `data.len()` are multiples of 4 and the range lies inside
    /// the buffer. Writes apply in call order before the next frame.
    pub fn write_range(
        self,
        host: &(impl StorageBufferContext + ?Sized),
        offset: u64,
        data: &[u8],
    ) -> Result<(), BufferError> {
        host.storage_buffer_backend()
            .ok_or(BufferError::Unavailable)?
            .0
            .storage_buffer_write_range(self, offset, data)
    }

    /// The size in bytes this buffer was created with.
    pub fn size(self, host: &(impl StorageBufferContext + ?Sized)) -> Result<u64, BufferError> {
        host.storage_buffer_backend()
            .ok_or(BufferError::Unavailable)?
            .0
            .storage_buffer_size(self)
    }

    /// Immediately free this buffer and retire the handle ([`BufferError::Stale`] thereafter).
    /// Writes queued for it and not yet applied are dropped.
    pub fn release(self, host: &(impl StorageBufferContext + ?Sized)) -> Result<(), BufferError> {
        host.storage_buffer_backend()
            .ok_or(BufferError::Unavailable)?
            .0
            .storage_buffer_release(self)
    }

    /// Copy `len` bytes from `offset` of this buffer into a staging buffer, in the command order
    /// of the call (vanilla's `CommandEncoder.copyToBuffer(source, target)`, which is an encoder
    /// command at the caller's point), and return the [`ReadbackTicket`] that will hold them.
    /// The buffer must have been created with [`StorageBufferDescriptor::copy_src`], `offset` and
    /// `len` are multiples of 4 (`wgpu::COPY_BUFFER_ALIGNMENT`), `len` is not zero and the range
    /// lies inside the buffer.
    ///
    /// Allowed only where compute is recorded: inside a frame-graph compute node's `record`
    /// callback ([`crate::FrameGraphContext::add_compute_pass`]). The copy sees the writes of the
    /// dispatches recorded before this call in that node and none of those recorded after it.
    /// Anywhere else it is refused with [`BufferError::NotInFrame`].
    ///
    /// Nothing waits: the bytes come back through [`ReadbackTicket::poll_readback`] a frame or
    /// more later. Running out of memory for the staging buffer is
    /// [`BufferError::ExceedsDeviceLimit`]; there is no cap on tickets beyond the device's.
    pub fn request_readback(
        self,
        host: &(impl StorageBufferContext + ?Sized),
        offset: u64,
        len: u64,
    ) -> Result<ReadbackTicket, BufferError> {
        host.storage_buffer_backend()
            .ok_or(BufferError::Unavailable)?
            .0
            .storage_buffer_request_readback(self, offset, len)
    }

    /// The range `offset..offset + length` of this buffer, to bind with
    /// [`crate::DrawBindings::storage_buffer`] (`GpuBuffer.slice`). The range is checked when
    /// it is bound, not here.
    pub const fn slice(self, offset: u64, length: u64) -> StorageBufferSlice {
        StorageBufferSlice {
            buffer: self,
            offset,
            length,
        }
    }
}

impl crate::__private::game::Fields for StorageBufferRef {
    type Fields<'a> = crate::__private::game::ModelRefFields;
    fn fields(&self) -> Self::Fields<'_> {
        crate::__private::game::ModelRefFields {
            generation: self.0,
            slot: self.1,
        }
    }
}

/// A pending readback of a storage buffer range, from [`StorageBufferRef::request_readback`].
///
/// Poll it each frame with [`Self::poll_readback`]. It is freed when its bytes are returned, by
/// [`Self::release`], or when its mod unloads.
// no wire: process-local handle to a pending GPU readback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the engine creates tickets.
pub struct ReadbackTicket(pub(crate) u64);

impl ReadbackTicket {
    pub(crate) fn from_id(id: u64) -> Result<Self, &'static str> {
        if id == 0 {
            return Err("unpublished readback ticket");
        }
        Ok(Self(id))
    }

    /// The bytes of the requested range if the GPU has finished the copy (vanilla's `GpuFence`
    /// checked without waiting, then `GpuBuffer.map(read)`): `Ok(None)` while it is in flight,
    /// then `Ok(Some(bytes))` once, after which the ticket is spent. Never waits on the GPU.
    /// A spent, released or unknown ticket, a copy the frame did not run, and a lost device are
    /// [`BufferError::Unavailable`].
    pub fn poll_readback(
        self,
        host: &(impl StorageBufferContext + ?Sized),
    ) -> Result<Option<Vec<u8>>, BufferError> {
        host.storage_buffer_backend()
            .ok_or(BufferError::Unavailable)?
            .0
            .readback_poll(self)
    }

    /// Drop a ticket that is no longer wanted and free its staging buffer. An unknown or spent
    /// ticket is [`BufferError::Unavailable`].
    pub fn release(self, host: &(impl StorageBufferContext + ?Sized)) -> Result<(), BufferError> {
        host.storage_buffer_backend()
            .ok_or(BufferError::Unavailable)?
            .0
            .readback_release(self)
    }
}

impl crate::__private::game::Fields for ReadbackTicket {
    type Fields<'a> = crate::__private::game::ModelRefFields;
    fn fields(&self) -> Self::Fields<'_> {
        crate::__private::game::ModelRefFields {
            generation: self.0,
            slot: 0,
        }
    }
}

/// A byte range of a [`StorageBufferRef`] (vanilla's `GpuBufferSlice`).
///
/// A binding takes a range of at most the device's `max_storage_buffer_binding_size` bytes, its
/// offset a multiple of `min_storage_buffer_offset_alignment`, its length a non-zero multiple of
/// 4. The buffer itself may be as large as `max_buffer_size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client storage buffer range.
pub struct StorageBufferSlice {
    /// The buffer the range lies in.
    pub buffer: StorageBufferRef,
    /// First byte of the range.
    pub offset: u64,
    /// Length of the range in bytes.
    pub length: u64,
}

/// What [`crate::prelude::ClientRenderClientHost::create_storage_buffer`] makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client storage buffer descriptor.
pub struct StorageBufferDescriptor {
    /// Size in bytes: at least 4, a multiple of 4 and at most the device's `max_buffer_size`.
    pub size: u64,
    /// Whether the buffer can also hold indirect draw or dispatch arguments (`wgpu`'s
    /// `BufferUsages::INDIRECT`; vanilla's `USAGE_INDIRECT_PARAMETERS`). Off by default.
    pub indirect: bool,
    /// Whether the buffer can be read back ([`StorageBufferRef::request_readback`]; `wgpu`'s
    /// `BufferUsages::COPY_SRC`; vanilla's `USAGE_COPY_SRC`). Off by default.
    pub copy_src: bool,
}

impl StorageBufferDescriptor {
    /// A storage buffer of `size` bytes.
    pub const fn new(size: u64) -> Self {
        Self {
            size,
            indirect: false,
            copy_src: false,
        }
    }

    /// Let the buffer also hold indirect draw or dispatch arguments.
    pub const fn indirect(mut self, indirect: bool) -> Self {
        self.indirect = indirect;
        self
    }

    /// Let the buffer be read back with [`StorageBufferRef::request_readback`].
    pub const fn copy_src(mut self) -> Self {
        self.copy_src = true;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_descriptor_starts_direct_and_a_slice_names_its_range() {
        let desc = StorageBufferDescriptor::new(64);
        assert_eq!(
            (desc.size, desc.indirect, desc.copy_src),
            (64, false, false)
        );
        assert!(desc.indirect(true).indirect);
        assert!(desc.copy_src().copy_src);
        let buffer = StorageBufferRef::from_parts(3, 1).unwrap();
        let slice = buffer.slice(16, 32);
        assert_eq!((slice.buffer, slice.offset, slice.length), (buffer, 16, 32));
        assert!(StorageBufferRef::from_parts(0, 0).is_err());
    }

    #[test]
    fn storage_buffer_ref_routes_through_backend_and_refuses_without_one() {
        struct Bare;
        impl crate::Host for Bare {}
        crate::impl_host_areas!(Bare);
        impl crate::ClientHost for Bare {}
        crate::impl_client_host_areas!(Bare);

        let buf = StorageBufferRef::from_parts(1, 0).unwrap();
        assert_eq!(
            buf.write_range(&Bare, 0, &[0; 16]),
            Err(BufferError::Unavailable)
        );
        assert_eq!(buf.size(&Bare), Err(BufferError::Unavailable));
        assert_eq!(buf.release(&Bare), Err(BufferError::Unavailable));
        assert_eq!(
            buf.request_readback(&Bare, 0, 16),
            Err(BufferError::Unavailable)
        );
        let ticket = ReadbackTicket::from_id(1).unwrap();
        assert!(ReadbackTicket::from_id(0).is_err());
        assert_eq!(ticket.poll_readback(&Bare), Err(BufferError::Unavailable));
        assert_eq!(ticket.release(&Bare), Err(BufferError::Unavailable));

        struct Recording {
            written: std::cell::RefCell<Option<(u64, Vec<u8>)>>,
        }
        impl crate::Host for Recording {
            fn backend(&self) -> Option<crate::BackendRef<'_>> {
                Some(crate::BackendRef::of(self))
            }
        }
        crate::impl_host_areas!(Recording);
        impl crate::ClientHost for Recording {}
        crate::impl_client_host_areas!(Recording);
        impl crate::Backend for Recording {}
        crate::impl_backend_areas!(Recording, except(client_render));
        impl crate::prelude::ClientRenderClientBackend for Recording {
            fn storage_buffer_write_range(
                &self,
                _: StorageBufferRef,
                offset: u64,
                data: &[u8],
            ) -> Result<(), BufferError> {
                *self.written.borrow_mut() = Some((offset, data.to_vec()));
                Ok(())
            }
            fn storage_buffer_size(&self, _: StorageBufferRef) -> Result<u64, BufferError> {
                Ok(64)
            }
            fn storage_buffer_release(&self, _: StorageBufferRef) -> Result<(), BufferError> {
                Err(BufferError::Stale)
            }
            fn storage_buffer_request_readback(
                &self,
                _: StorageBufferRef,
                offset: u64,
                len: u64,
            ) -> Result<ReadbackTicket, BufferError> {
                if offset == 0 && len == 16 {
                    Ok(ReadbackTicket::from_id(7).unwrap())
                } else {
                    Err(BufferError::NotInFrame)
                }
            }
            fn readback_poll(
                &self,
                ticket: ReadbackTicket,
            ) -> Result<Option<Vec<u8>>, BufferError> {
                Ok((ticket.0 == 7).then(|| vec![9; 4]))
            }
            fn readback_release(&self, _: ReadbackTicket) -> Result<(), BufferError> {
                Err(BufferError::Unavailable)
            }
        }

        let host = Recording {
            written: std::cell::RefCell::new(None),
        };
        buf.write_range(&host, 8, &[1, 2, 3, 4]).unwrap();
        assert_eq!(*host.written.borrow(), Some((8, vec![1, 2, 3, 4])));
        assert_eq!(buf.size(&host), Ok(64));
        assert_eq!(buf.release(&host), Err(BufferError::Stale));
        let ticket = buf.request_readback(&host, 0, 16).unwrap();
        assert_eq!(ticket.poll_readback(&host), Ok(Some(vec![9; 4])));
        assert_eq!(
            buf.request_readback(&host, 4, 16),
            Err(BufferError::NotInFrame)
        );
        assert_eq!(ticket.release(&host), Err(BufferError::Unavailable));
    }
}
