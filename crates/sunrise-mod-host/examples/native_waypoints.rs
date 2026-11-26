//! Playable native API example. Run `cargo run -p sunrise-mod-host --example native_waypoints --`
//! with prepared resources and a disposable game directory. In an owned single-player save,
//! use `/waypoint_set` to place a cyan marker. The most recent death appears in red after respawn.
use mod_api::{
    ClientHost, ClientTickPostEvent, Hooks, ModEntry, RegisterClientCommandsEvent,
    SubmitCustomGeometryEvent, WorldDepth, WorldLine,
};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::{fs, io};

#[derive(Default)]
struct Markers {
    key: Option<(String, String)>,
    manual: Option<[i32; 3]>,
    death: Option<[i32; 3]>,
}

#[derive(Default)]
struct Waypoints(Arc<Mutex<Markers>>);

/// Escape path separators and Windows-reserved characters so an imported save
/// folder or namespaced dimension maps to one filename component.
fn escape(part: &str) -> String {
    part.replace('%', "%25")
        .replace('/', "%2F")
        .replace(':', "%3A")
        .replace('\\', "%5C")
}

fn waypoint_path(instance_dir: &Path, save: &str, dimension: &str) -> Option<PathBuf> {
    if save.is_empty() || matches!(save, "." | "..") || dimension.is_empty() {
        return None;
    }
    let (save, dimension) = (escape(save), escape(dimension));
    Some(
        instance_dir
            .join("sunrise-mod-data")
            .join("waypoints")
            .join(save)
            .join(format!("{dimension}.json")),
    )
}

fn read_waypoint(path: &Path) -> Option<[i32; 3]> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).ok(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => {
            log::warn!("Could not load the saved waypoint: {error}");
            None
        }
    }
}

fn write_waypoint(path: &Path, position: [i32; 3]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(&position).expect("integer coordinates serialize");
    fs::write(path, bytes)
}

#[mod_api::__subscriptions]
impl Hooks for Waypoints {
    fn register_client_commands(
        &self,
        _: &mut dyn ClientHost,
        event: &mut RegisterClientCommandsEvent<'_>,
    ) {
        let state = Arc::clone(&self.0);
        event
            .commands
            .push(
                mod_api::Command::new("waypoint_set").executes_client(move |host, _| {
                    let dimension = host.dimension();
                    let target = host
                        .client_session()
                        .client_server()
                        .and_then(|server| {
                            let save = server.save_folder?;
                            let path = waypoint_path(&host.instance_dir(), save, dimension)?;
                            Some((save.to_owned(), path))
                        })
                        .zip(host.local_player().and_then(|player| {
                            player
                                .entity()
                                .position(host)
                                .map(|pos| pos.map(|axis| axis.floor() as i32))
                        }));
                    if let Some(((save, path), position)) = target {
                        match write_waypoint(&path, position) {
                            Ok(()) => {
                                let mut markers = state.lock().unwrap();
                                markers.key = Some((save, dimension.to_owned()));
                                markers.manual = Some(position);
                                return Some(1);
                            }
                            Err(error) => log::warn!("Could not save the waypoint: {error}"),
                        }
                    }
                    None
                }),
            );
    }

    fn client_tick_post(&self, host: &mut dyn ClientHost, _: &mut ClientTickPostEvent) {
        let dimension = host.dimension();
        let key = host
            .client_session()
            .client_server()
            .and_then(|server| server.save_folder.map(|save| (save, dimension)));
        let mut markers = self.0.lock().unwrap();
        let current = markers
            .key
            .as_ref()
            .map(|(save, dim)| (save.as_str(), dim.as_str()));
        if current != key {
            markers.manual = key.and_then(|(save, dim)| {
                let path = waypoint_path(&host.instance_dir(), save, dim)?;
                read_waypoint(&path)
            });
            markers.key = key.map(|(save, dim)| (save.to_owned(), dim.to_owned()));
        }
        markers.death = host
            .local_player()
            .and_then(|player| player.last_death_location(host))
            .filter(|death| death.dimension().to_string() == dimension)
            .map(|death| {
                let pos = death.pos();
                [pos.x, pos.y, pos.z]
            });
    }

    fn client_level_unload(
        &self,
        _: &mut dyn ClientHost,
        _: &mut mod_api::ClientLevelUnloadEvent<'_>,
    ) {
        *self.0.lock().unwrap() = Markers::default();
    }

    fn submit_custom_geometry(
        &self,
        _: &dyn ClientHost,
        event: &mut SubmitCustomGeometryEvent<'_>,
    ) {
        let markers = self.0.lock().unwrap();
        for (point, color) in [(markers.manual, 0xff00ffff), (markers.death, 0xffff3030)] {
            if let Some([x, y, z]) = point {
                let from = [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5];
                let to = [from[0], from[1] + 8.0, from[2]];
                let _ = event.draw.lines(&[WorldLine::new(
                    from,
                    to,
                    color,
                    3.0,
                    WorldDepth::ThroughWalls,
                )]);
            }
        }
    }
}

fn main() {
    let startup = sunrise_client::entry::prepare();
    sunrise_mod_host::main_with(
        vec![ModEntry::new("waypoints", Box::<Waypoints>::default())],
        &[],
        startup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waypoint_files_are_scoped_to_the_save_and_dimension() {
        let root = Path::new("/instance");
        let overworld = waypoint_path(root, "Test World", "minecraft:overworld").unwrap();
        assert_eq!(
            overworld,
            root.join("sunrise-mod-data/waypoints/Test World/minecraft%3Aoverworld.json")
        );
        assert_ne!(
            Some(overworld.clone()),
            waypoint_path(root, "Test World", "minecraft:the_nether")
        );
        assert_ne!(
            Some(overworld),
            waypoint_path(root, "Test World (2)", "minecraft:overworld")
        );
        assert_ne!(
            waypoint_path(root, "Test World", "example:foo/bar"),
            waypoint_path(root, "Test World", "example:foo%2Fbar")
        );
        assert_eq!(waypoint_path(root, "", "minecraft:overworld"), None);
        assert_eq!(waypoint_path(root, ".", "minecraft:overworld"), None);
        assert_eq!(waypoint_path(root, "..", "minecraft:overworld"), None);
        assert_eq!(waypoint_path(root, "Test World", ""), None);
        assert!(waypoint_path(root, "a:b\\c", "minecraft:overworld").is_some());
        assert_ne!(
            waypoint_path(root, "a:b", "minecraft:overworld"),
            waypoint_path(root, "a%3Ab", "minecraft:overworld")
        );
    }
}
