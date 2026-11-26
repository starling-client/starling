//! Starling's part of arena.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

/// The bytes past which pages of `page_bytes` stop growing: all of them here
/// (see the 64-bit Android one).
#[cfg(not(all(target_os = "android", target_pointer_width = "64")))]
pub(super) fn page_ceiling(page_bytes: u64) -> u64 {
    page_bytes
}

/// `try_create` for a page's buffer, in a span of its own: a phone's driver can take hundreds of
/// milliseconds here. Adapted from Vasysik/sunrise-android (`arena_buffer_create`), MIT.
pub(super) fn create_page(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> Option<wgpu::Buffer> {
    crate::spans::time("arena_buffer_create", || {
        try_create(device, label, size, usage)
    })
}

impl Arena {
    /// World-space ray-tracing positions (`BLAS_INPUT`): the triangle soup `rt`
    /// builds one BLAS per section over. Never compacted: a range stays valid
    /// until its section is released, unlike the render arenas' ranges.
    pub fn ray_tracing(device: &wgpu::Device) -> Self {
        Self::with_usage(
            device,
            12,
            1 << 16,
            wgpu::BufferUsages::BLAS_INPUT,
            PAGE_BYTES,
        )
    }
}

impl Arena {
    /// Makes the first page's buffer ahead of the first upload (on the level
    /// loading screen), where the arena has no page and no spare: the first
    /// sections then do not wait for the driver to allocate it.
    pub fn prewarm(&mut self, device: &wgpu::Device) {
        if !self.keeps_spare || self.spare.is_some() || self.pages.iter().any(Option::is_some) {
            return;
        }
        if let Some((buffer, false)) = self.create(device, self.initial_capacity, false) {
            self.spare = Some(buffer);
        }
    }

    /// The vertices a page grows to at most (`growth_limit`, within `page_limit`).
    pub(super) fn ceiling(&self) -> u64 {
        self.growth_limit
            .map_or(self.page_limit, |limit| limit.min(self.page_limit))
    }

    /// The spare buffer (`spare`) for a page of `capacity` vertices.
    pub(super) fn take_spare(&mut self, capacity: u64) -> Option<wgpu::Buffer> {
        if capacity == self.initial_capacity {
            self.spare.take()
        } else {
            None
        }
    }

    /// An emptied page's buffer, unless it becomes the spare (`spare`): one in
    /// video memory of the first page's size, where none is kept yet.
    pub(super) fn keep_spare(&mut self, page: Page) -> Option<wgpu::Buffer> {
        if self.keeps_spare
            && self.spare.is_none()
            && !page.host
            && page.capacity == self.initial_capacity
        {
            self.spare = Some(page.buffer);
            None
        } else {
            Some(page.buffer)
        }
    }
}
