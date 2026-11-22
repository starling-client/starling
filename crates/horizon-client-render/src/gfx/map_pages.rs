use crate::hud::{MAP_PAGE_COUNT, MAP_PAGE_SIDE, MAP_SLOTS, MAP_TILE_SIDE, MAPS_PER_PAGE};

pub(crate) fn layouts() -> [wgpu::BindGroupLayoutEntry; MAP_PAGE_COUNT] {
    std::array::from_fn(|page| wgpu::BindGroupLayoutEntry {
        binding: 8 + page as u32,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    })
}

pub(crate) fn bindings(
    views: [&wgpu::TextureView; MAP_PAGE_COUNT],
) -> [wgpu::BindGroupEntry<'_>; MAP_PAGE_COUNT] {
    std::array::from_fn(|page| wgpu::BindGroupEntry {
        binding: 8 + page as u32,
        resource: wgpu::BindingResource::TextureView(views[page]),
    })
}

fn texture(device: &wgpu::Device, side: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Map tiles"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// At most four independent allocations, without copying old pages or doubling
/// their memory when another page is needed.
pub(super) struct MapPages {
    empty: wgpu::TextureView,
    pages: [Option<(wgpu::Texture, wgpu::TextureView)>; MAP_PAGE_COUNT],
    allocation_failed: bool,
}

impl MapPages {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        Self {
            empty: texture(device, 1).create_view(&Default::default()),
            pages: Default::default(),
            allocation_failed: false,
        }
    }

    pub(super) fn views(&self) -> [&wgpu::TextureView; MAP_PAGE_COUNT] {
        std::array::from_fn(|page| {
            self.pages[page]
                .as_ref()
                .map_or(&self.empty, |(_, view)| view)
        })
    }

    pub(super) fn allocation_failed(&self) -> bool {
        self.allocation_failed
    }

    pub(super) fn clear(&mut self) -> bool {
        self.allocation_failed = false;
        let changed = self.pages.iter().any(Option::is_some);
        if changed {
            self.pages = Default::default();
        }
        changed
    }

    pub(super) fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        tiles: &[(usize, Vec<u8>)],
    ) -> bool {
        let mut bindings_changed = false;
        if tiles.is_empty() {
            return bindings_changed;
        }
        // A frame may mark the same tile dirty twice. Upload at most one
        // payload per slot even in that case.
        let mut pending = [None; MAP_SLOTS];
        for (slot, rgba) in tiles {
            if let Some(entry) = pending.get_mut(*slot)
                && rgba.len() == (MAP_TILE_SIDE * MAP_TILE_SIDE * 4) as usize
            {
                *entry = Some(rgba.as_slice());
            }
        }
        for (slot, rgba) in pending
            .into_iter()
            .enumerate()
            .filter_map(|(slot, rgba)| Some((slot, rgba?)))
        {
            let page = &mut self.pages[slot / MAPS_PER_PAGE];
            if page.is_none() && !self.allocation_failed {
                let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
                let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
                let texture = texture(device, MAP_PAGE_SIDE);
                let internal = pollster::block_on(internal.pop());
                let memory = pollster::block_on(memory.pop());
                if let Some(error) = internal.or(memory) {
                    self.allocation_failed = true;
                    log::warn!("Map pages: could not allocate a page: {error}");
                } else {
                    let view = texture.create_view(&Default::default());
                    *page = Some((texture, view));
                    bindings_changed = true;
                }
            }
            let Some((texture, _)) = page else { continue };
            let tile = (slot % MAPS_PER_PAGE) as u32;
            let row = MAP_PAGE_SIDE / MAP_TILE_SIDE;
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: (tile % row) * MAP_TILE_SIDE,
                        y: (tile / row) * MAP_TILE_SIDE,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(MAP_TILE_SIDE * 4),
                    rows_per_image: Some(MAP_TILE_SIDE),
                },
                wgpu::Extent3d {
                    width: MAP_TILE_SIDE,
                    height: MAP_TILE_SIDE,
                    depth_or_array_layers: 1,
                },
            );
        }
        if bindings_changed {
            log::info!(
                "Map pages: {} allocated",
                self.pages.iter().flatten().count()
            );
        }
        bindings_changed
    }
}
