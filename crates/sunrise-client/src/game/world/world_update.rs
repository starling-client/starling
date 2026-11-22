//! The frame's world update (`update_world`): what arrived from the server, the chunks that
//! streamed in and the meshes handed to the renderer, the columns that unload (their entities
//! and block entities saved out first); and far terrain (`prepare_lod`, `clear_lod`).
use super::*;

impl Game<'_> {
    pub(super) fn clear_lod(&mut self) {
        // The samplers waiting on the server stop first: dropping the far
        // terrain joins them.
        if let Some(remote) = &self.remote {
            remote.session.cancel_far_terrain();
        }
        self.lod = None;
        self.lod_source_attempted = false;
        self.lod_observe.clear();
        self.lod_blue_map = None;
        self.lod_covered_at = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.clear_lods();
        }
    }
    pub(super) fn prepare_lod(&mut self) {
        let enabled = self.far_terrain_choice().0 > 0
            && self.renderer.is_some()
            && self.player.is_some()
            && self.title.is_none();
        if !enabled {
            if self.lod.is_some() || self.lod_source_attempted {
                self.clear_lod();
            }
            return;
        }
        // Another level: its own kept chunks. (Another world at the same
        // address, a proxy's next server, comes through a new login; a seed
        // changed at a respawn of the same level is a plugin hiding it.)
        if self
            .lod_level
            .as_ref()
            .is_some_and(|level| self.remote.as_ref().is_some_and(|r| r.level != *level))
        {
            self.clear_lod();
        }
        if self.lod_source_attempted {
            return;
        }
        self.lod_source_attempted = true;
        self.lod_level = None;
        let dim = self.dim;
        let source = match (&self.integrated, self.server_far_terrain(), &self.remote) {
            (Some(server), ..) => server
                .ask(|reply| super::owner_link::OwnerRequest::LodSource(dim, reply))
                .flatten(),
            (None, Some(offer), Some(remote)) => {
                let states = self.res.facts().states.len() as u32;
                let biomes = self.res.biomes().index.clone();
                let source: Arc<dyn sunrise_client::streaming::LodSource> =
                    remote.session.far_terrain(offer, biomes, states);
                Some(source)
            }
            // A friend's world has a new code at every share: nothing to keep.
            (None, None, Some(remote))
                if remote
                    .session
                    .address
                    .starts_with(sunrise_client::net::FRIEND) =>
            {
                None
            }
            // No offer from a server: draw what it sent, on this visit and before.
            (None, None, Some(remote)) => std::env::current_dir().ok().map(|home| {
                use sunrise_client::server_terrain as terrain;
                let address = &remote.session.address;
                let root = home.join("server-terrain");
                let dir = terrain::level_dir(&root, address, remote.hashed_seed, &remote.level);
                self.lod_level = Some(remote.level.clone());
                // Past them, the server's BlueMap, if the player named it.
                let maps = root.join(terrain::server_folder(address)).join("BlueMap");
                let settings = &self.settings.sunrise.blue_map;
                let blue_map = settings.server(address).map(|server| {
                    use sunrise_client::bluemap;
                    // The map's address only at debug: logs land in crash reports.
                    log::info!("Sunrise: far terrain also draws {}'s BlueMap", remote.level);
                    log::debug!("Sunrise: BlueMap at {}", server.url);
                    let map = Arc::new(bluemap::BlueMap::new(
                        &server.url,
                        &remote.level,
                        server.maps.clone(),
                        maps,
                        settings.radius_blocks(),
                        bluemap::Palette::from_game(self.pack.assets.registry, self.res.biomes()),
                    ));
                    map.start();
                    map
                });
                self.lod_blue_map = blue_map.clone();
                terrain::ServerTerrain::new(dir, blue_map) as _
            }),
            _ => Some(self.streaming.lod_source()),
        };
        if let Some(source) = source {
            self.lod = Some(sunrise_client::lod::Lod::new(source, self.worker_context()));
            self.lod_observe.extend(self.world.chunks.keys().copied());
        }
    }
    pub(super) fn update_world(&mut self) {
        if let Err(e) = sunrise_client::spans::time("poll_remote", || self.poll_remote()) {
            log::warn!("Sunrise: {e}");
        }
        if let Err(e) = self.advance_studio_replay() {
            log::warn!("Sunrise Studio replay: {e}");
        }
        // The renderer laid translucent quads out again (a shader pack's flat terrain came
        // or went) and dropped every section and far tile: meshed again, as for a new one.
        if self
            .renderer
            .as_mut()
            .is_some_and(Renderer::take_dropped_sections)
        {
            self.clear_lod();
            self.all_sections_changed();
        }
        sunrise_client::spans::time("prepare_lod", || self.prepare_lod());
        // The columns the single-player server changed in bulk without sending them, which far terrain still drew
        // as this window last saw them: summarized again (taken every tick, far terrain on or off, so none waits).
        if self.remote.is_some() {
            let changed = sunrise_client::lan::handoff::take_changed();
            if let Some(lod) = &mut self.lod {
                for chunk in changed {
                    let column = (chunk.x, chunk.z);
                    if self.world.chunks.contains_key(&column) {
                        continue;
                    }
                    lod.observe(chunk, self.res.facts());
                }
            }
        }
        self.observe_light_edits();
        self.ask_edited_light_emission();
        if let (Some(map), Some(p), Some(lod)) = (&self.lod_blue_map, &self.player, &mut self.lod) {
            map.set_center(p.position.x.floor() as i32, p.position.z.floor() as i32);
            for change in map.take_changed() {
                lod.refresh(change.area, change.widths);
            }
        }
        if let Some(p) = &self.player {
            self.streaming.center = (
                (p.position.x.floor() as i32) >> 4,
                (p.position.z.floor() as i32) >> 4,
            );
            self.streaming.center_y = p.position.y.floor() as i32;
        }
        sunrise_client::spans::time("tick_pregen", || self.tick_pregen());
        // A headless game has nothing to draw sections with: like a server's, they aren't
        // meshed while it has no renderer.
        if self.options.headless {
            self.streaming.mesh = self.renderer.is_some();
        }
        if self.defer_chunk_disk_save() {
            return;
        }
        let areas = self.streaming.extra.len();
        let dim = self.dim;
        self.streaming.extra.extend(
            self.server_trips
                .iter()
                .flat_map(|trip| trip.spawn_destination_areas(dim)),
        );
        let waiting = std::mem::take(&mut self.spawn_wait);
        self.streaming
            .extra
            .extend(waiting.into_iter().map(|chunk| (chunk, 0)));
        self.streaming.upload_backlog = self.pending_uploads.len();
        let plan = sunrise_client::spans::time("streaming_plan", || {
            let mut step = self.streaming.begin_plan_update(&mut self.world);
            loop {
                match step {
                    sunrise_client::streaming::PlanStep::Ready(plan) => break plan,
                    sunrise_client::streaming::PlanStep::Save { chunk, resume } => {
                        self.native_late_full_chunk_snapshot(&chunk);
                        self.streaming.save_finished(chunk);
                        step = self.streaming.resume_plan_update(&mut self.world, resume);
                    }
                }
            }
        });
        let (commit, protos) = self.streaming.prepare_commit(&mut self.world, plan);
        self.native_proto_snapshots(&protos);
        self.streaming.submit_protos(protos);
        let (mut uploads, unloaded) = sunrise_client::spans::time("streaming", || {
            self.streaming.complete_commit(&mut self.world, commit)
        });
        // Sections chosen to mesh wait for the mods' `section_visibility` (none without a mod).
        self.prepare_section_visibility();
        self.streaming.extra.truncate(areas);
        // The viewed levels' meshing, queued after the player's level's (nothing while no view
        // names one).
        if self.streaming.mesh
            && let Some(remote) = &mut self.remote
        {
            let streaming = &self.streaming;
            remote
                .viewed
                .mesh_frame(|| streaming.viewed_mesh_source(), std::time::Instant::now());
        }
        let dirty: Vec<_> = unloaded
            .iter()
            .filter(|c| self.world.modified.remove(&(c.x, c.z)))
            .cloned()
            .collect();
        let modified = sunrise_client::spans::time("with_entities", || self.with_entities(&dirty));
        self.native_full_chunk_snapshots(&modified);
        if !modified.is_empty()
            && let Err(e) = sunrise_client::spans::time("save_chunks", || {
                self.save
                    .save_later(&modified, self.pack.assets.registry, world_biomes!(self))
            })
        {
            log::warn!("Sunrise: failed to save unloaded chunks: {e}");
        }
        sunrise_client::spans::time("spawn_fresh_chunks", || self.spawn_fresh_chunks());
        self.ask_loaded_light_emission();
        // Vanilla saves the chunk before its unload hook and drops its block entities afterwards.
        sunrise_client::spans::time("mods_chunks_changed", || {
            self.native_chunks_loaded();
            self.native_chunks_unloaded(&unloaded);
        });
        for chunk in &unloaded {
            self.world.modified.unloaded(&(chunk.x, chunk.z));
        }
        if let Some(lod) = &mut self.lod {
            self.lod_observe
                .extend(uploads.iter().map(|upload| (upload.key.0, upload.key.2)));
            for chunk in &unloaded {
                self.lod_observe.remove(&(chunk.x, chunk.z));
            }
            // Unloaded columns first (their summaries are all that remains),
            // then a few resident ones (an edit copies a chunk the worker
            // still holds). The worker summarizes them.
            sunrise_client::spans::time("lod_observe", || {
                for chunk in &unloaded {
                    lod.observe(chunk.clone(), self.res.facts());
                }
                while lod.observing() < sunrise_client::lod::OBSERVE_AHEAD
                    && let Some(&key) = self.lod_observe.iter().next()
                {
                    self.lod_observe.remove(&key);
                    if let Some(chunk) = self.world.chunks.get(&key) {
                        lod.observe(chunk.clone(), self.res.facts());
                    }
                }
            });
        }
        for chunk in &unloaded {
            self.world.modified.unloaded(&(chunk.x, chunk.z));
        }
        sunrise_client::spans::time("unload_entities", || self.unload_entities(&unloaded));
        if !unloaded.is_empty() {
            let gone: sunrise_client::fast_hash::FxHashSet<(i32, i32)> =
                unloaded.iter().map(|c| (c.x, c.z)).collect();
            sunrise_client::spans::time("unload_block_entities", || {
                for &column in &gone {
                    self.world.block_entities.unload_column(column);
                }
            });
            self.world
                .block_names
                .retain(|p, _| !gone.contains(&(p.0 >> 4, p.2 >> 4)));
        }
        // LevelChunk.registerTickContainerInLevel: a chunk that became
        // resident has its tick containers before a command can schedule into
        // it, even in a frozen level that runs no world tick. The unloaded
        // chunks above were saved with their ticks first.
        if self.remote.is_none() {
            sunrise_client::spans::time("sync_tick_chunks", || self.sync_tick_chunks());
        }
        let (far_distance, far_coarseness) =
            sunrise_client::spans::time("far_terrain_choice", || self.far_terrain_choice());
        // Terrain still on its way from the server (single player's integrated one
        // too): the view's outermost columns not all in, or its events not all handled.
        let radius = match &self.remote {
            // As far as the server fills it: its view distance and one.
            Some(r) => self.streaming.radius.min(r.view_distance + 1),
            None => self.streaming.radius,
        };
        let view = (self.streaming.center, radius);
        let has = |column: (i32, i32)| self.world.chunks.contains_key(&column);
        let now = Instant::now();
        let arriving = {
            let _span = sunrise_client::spans::guard("view_arrival");
            self.view_arrival.arriving(view, now, has) || self.remote_backlog() > 0
        };
        // Which far tiles a zoom magnifies (`lod::ZoomAim`), read before
        // `self.renderer.as_mut()` below borrows self mutably for the rest of
        // this function. Cheap: trigonometry on one angle, or None when the
        // view is not zoomed.
        let lod_aim = self
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z])
            .and_then(|camera| self.lod_aim(camera));
        if let Some(renderer) = self.renderer.as_mut() {
            let g = &self.settings.sunrise;
            let budgeted = g.chunk_updates > 0 || g.upload_budget_kib > 0;
            let mut raw_flight = self.flight.as_mut().filter(|flight| flight.raw);
            let vertex_bytes = |mesh: &sunrise_client::mesher::SectionMesh| {
                ((mesh.solid.len() + mesh.translucent.len())
                    * std::mem::size_of::<sunrise_client::mesher::TerrainVertex>())
                    as u64
            };
            // The meshes, once copied, are freed by a worker (`Streaming::free`).
            if !budgeted && self.pending_uploads.is_empty() && uploads.len() <= AUTO_UPLOADS_AT_ONCE
            {
                sunrise_client::spans::time("upload", || {
                    for upload in &uploads {
                        if self.streaming.can_publish_mesh(&self.world, upload) {
                            renderer.upload(upload.key, &upload.mesh);
                            if let Some(flight) = raw_flight.as_deref_mut() {
                                flight.note_mesh_uploads(1, vertex_bytes(&upload.mesh));
                            }
                        }
                    }
                });
                self.streaming.free(uploads);
            } else {
                // Sunrise's chunk-update and upload budgets: the nearest
                // sections go first, the rest wait for later frames; a newer
                // mesh of a waiting section replaces it, and sections of
                // chunks no longer loaded are dropped.
                let pending = &mut self.pending_uploads;
                let order_span = sunrise_client::spans::guard("order_uploads");
                let fresh: sunrise_client::fast_hash::FxHashSet<(i32, i32, i32)> =
                    uploads.iter().map(|upload| upload.key).collect();
                let current = |upload: &sunrise_client::streaming::MeshUpload| {
                    self.world
                        .chunks
                        .contains_key(&(upload.key.0, upload.key.2))
                        && self.streaming.can_publish_mesh(&self.world, upload)
                };
                // Filtering leaves the surviving waiting run in the same order.
                let mut spent: Vec<_> = pending
                    .extract_if(.., |upload| fresh.contains(&upload.key) || !current(upload))
                    .collect();
                spent.extend(uploads.extract_if(.., |upload| !current(upload)));
                let centre = self.streaming.center;
                // Section keys count from the world's bottom (`origin`'s min_y + y * 16).
                let bottom = f64::from(self.world.bottom_y());
                let cam_y = self
                    .player
                    .as_ref()
                    .map_or(4, |p| ((p.position.y - bottom) / 16.0).floor() as i32);
                let far = |k: &(i32, i32, i32)| {
                    (k.0 - centre.0).pow(2) + (k.1 - cam_y).pow(2) + (k.2 - centre.1).pow(2)
                };
                let order = Some((centre, cam_y));
                let priority = |upload: &sunrise_client::streaming::MeshUpload| {
                    std::cmp::Reverse(far(&upload.key))
                };
                let work = order_uploads(
                    pending,
                    &mut uploads,
                    &mut self.pending_uploads_merge,
                    self.pending_uploads_order == order,
                    priority,
                );
                self.pending_uploads_order = order;
                drop(order_span);
                let (limit, budget) = if budgeted {
                    (
                        g.chunk_updates as usize,
                        u64::from(g.upload_budget_kib) * 1024,
                    )
                } else {
                    (0, 0)
                };
                let (mut count, mut bytes) = (0, 0u64);
                // Without the player's budgets, uploads stop once they took this
                // frame's share (`UploadPace::budget`: a quarter of the recent frame
                // time, more with a backlog, at most 2 ms; at least one goes): a burst
                // of finished meshes spreads over frames, nearest first.
                let auto = (!budgeted).then(|| self.upload_pace.budget(pending.len(), arriving));
                let started = Instant::now();
                let upload_span = sunrise_client::spans::guard("upload_queue");
                while let Some(upload) = pending.last() {
                    let mesh = &upload.mesh;
                    if auto.is_some_and(|time| count > 0 && started.elapsed() >= time) {
                        break;
                    }
                    let size = vertex_bytes(mesh);
                    if (limit > 0 && count >= limit)
                        || (budget > 0 && bytes > 0 && bytes + size > budget)
                    {
                        break;
                    }
                    let upload = pending.pop().expect("checked above");
                    renderer.upload(upload.key, &upload.mesh);
                    spent.push(upload);
                    count += 1;
                    bytes += size;
                }
                drop(upload_span);
                if let Some(flight) = raw_flight {
                    flight.note_upload_queue_work(work);
                    flight.note_mesh_uploads(count, bytes);
                }
                self.streaming.free(spent);
            }
            renderer.remove_columns(unloaded.iter().map(|c| (c.x, c.z)));
            // Meshes in the works are the view's terrain while it is new, then edits'.
            let meshing = self.streaming.busy() || !self.pending_uploads.is_empty();
            let sampling = self.lod.as_ref().is_some_and(|lod| lod.sampling());
            renderer.set_terrain_loading(
                arriving || (meshing && self.view_arrival.recent(now)) || sampling,
            );
            if let (Some(lod), Some(player)) = (&mut self.lod, &self.player) {
                let lod_distance = sunrise_client::graphics::lod_distance_capped(
                    far_distance,
                    &renderer.info,
                    self.settings.sunrise.far_detail,
                );
                lod.set_near_loading(self.streaming.terrain_loading());
                lod.set_far_coarseness(far_coarseness);
                // The covered columns change only with the complete ones and
                // the render area: rebuilt then, not every frame.
                let at = (
                    renderer.lod_detail_revision(),
                    self.streaming.center,
                    self.streaming.radius,
                );
                let covered = (self.lod_covered_at != Some(at)).then(|| {
                    self.lod_covered_at = Some(at);
                    sunrise_client::spans::time("lod_covered_set", || {
                        renderer
                            .lod_detail_columns()
                            .filter(|&(x, z)| {
                                sunrise_client::streaming::in_view_distance(at.1, at.2, x, z)
                            })
                            .collect()
                    })
                });
                // Far terrain's finest tiles stand where full-detail terrain
                // ends: on a server, where it stops sending chunks, often well
                // inside the render distance. Thousands of chunk-sized tiles
                // between the two filled in long after the coarser ones.
                let detail = self.remote.as_ref().map_or(self.streaming.radius, |r| {
                    self.streaming.radius.min(r.sent_radius())
                });
                let camera = [player.position.x, player.position.y, player.position.z];
                let meshes = sunrise_client::spans::time("lod_update", || {
                    lod.update(camera, detail, lod_distance, covered, lod_aim)
                });
                renderer.queue_lods(meshes, lod.removed(), lod.take_cutout());
                // A finished job can hold hundreds of tiles: spread them
                // over frames rather than stall one.
                sunrise_client::spans::time("lod_upload", || {
                    renderer.upload_lods(std::time::Duration::from_millis(2))
                });
            }
        }
    }
}

/// Farthest first (the uploader pops from the back). Returns the number of
/// elements passed to a sort and to the merge, respectively.
fn order_uploads<T, K: Ord>(
    waiting: &mut Vec<T>,
    fresh: &mut Vec<T>,
    scratch: &mut Vec<T>,
    same_section: bool,
    priority: impl Fn(&T) -> K,
) -> [usize; 2] {
    if !same_section {
        waiting.append(fresh);
        if waiting.len() < 2 {
            return [0, 0];
        }
        waiting.sort_unstable_by_key(priority);
        return [waiting.len(), 0];
    }
    let sorted = if fresh.len() > 1 {
        fresh.sort_unstable_by_key(&priority);
        fresh.len()
    } else {
        0
    };
    if waiting
        .last()
        .zip(fresh.first())
        .is_none_or(|(old, new)| priority(old) <= priority(new))
    {
        waiting.append(fresh);
        return [sorted, 0];
    }

    // At equal distance, write the waiting mesh first so the fresh mesh is
    // popped first.
    let merged = waiting.len() + fresh.len();
    scratch.reserve(merged);
    let mut old = waiting.drain(..).peekable();
    let mut new = fresh.drain(..).peekable();
    while let (Some(a), Some(b)) = (old.peek(), new.peek()) {
        scratch.extend(if priority(a) <= priority(b) {
            old.next()
        } else {
            new.next()
        });
    }
    scratch.extend(old);
    scratch.extend(new);
    std::mem::swap(waiting, scratch);
    [sorted, merged]
}

#[cfg(test)]
mod tests {
    use super::order_uploads;
    use std::cmp::Reverse;

    #[test]
    fn stationary_upload_budget_gives_fresh_meshes_priority_on_ties() {
        let mut waiting = vec![(20, "far"), (4, "waiting")];
        let mut fresh = vec![(1, "nearest"), (9, "middle"), (4, "fresh")];
        let mut scratch = Vec::new();
        let priority = |&(distance, _): &(i32, &str)| Reverse(distance);
        order_uploads(&mut waiting, &mut fresh, &mut scratch, true, priority);
        // Pin the stationary queue's fresh-first tie policy: two uploads leave
        // the waiting mesh first in line next frame. The old sort_unstable_by_key
        // did not guarantee this tie order.
        let selected: Vec<_> = std::iter::from_fn(|| waiting.pop().map(|(_, name)| name))
            .take(2)
            .collect();
        assert_eq!(selected, ["nearest", "fresh"]);
        assert_eq!(waiting.pop(), Some((4, "waiting")));
        assert_eq!(waiting.pop(), Some((9, "middle")));
        assert_eq!(waiting.pop(), Some((20, "far")));
    }
}
