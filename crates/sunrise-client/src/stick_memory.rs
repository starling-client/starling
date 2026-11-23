//! Starling's stick mode per server ([`ServerStick`]) when the player asked the game to remember
//! it: `starling-servers.txt` in the game folder, one `address=mode` line each.
use crate::gamepad::ServerStick;
use std::path::Path;

pub const FILE: &str = "starling-servers.txt";

fn key(address: &str) -> String {
    address.trim().to_ascii_lowercase()
}

fn entries(dir: &Path) -> Vec<(String, ServerStick)> {
    std::fs::read_to_string(dir.join(FILE))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (address, mode) = line.rsplit_once('=')?;
            Some((key(address), ServerStick::parse(mode)))
        })
        .collect()
}

/// The mode remembered for `address`.
pub fn get(dir: &Path, address: &str) -> Option<ServerStick> {
    let address = key(address);
    entries(dir)
        .into_iter()
        .find(|(a, _)| *a == address)
        .map(|(_, mode)| mode)
}

/// Remember `mode` for `address` (in place of an earlier choice).
pub fn set(dir: &Path, address: &str, mode: ServerStick) {
    let address = key(address);
    let mut all = entries(dir);
    all.retain(|(a, _)| *a != address);
    all.push((address, mode));
    let text: String = all
        .iter()
        .map(|(a, m)| format!("{a}={}\n", m.name()))
        .collect();
    if let Err(e) = std::fs::write(dir.join(FILE), text) {
        log::warn!("Sunrise: {FILE} not written: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_server_keeps_its_own_mode() {
        let dir = std::env::temp_dir().join(format!("starling-sticks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(get(&dir, "pika-network.net"), None);
        set(&dir, "Pika-Network.net", ServerStick::Free);
        set(&dir, "play.example.org:25570", ServerStick::Keys);
        set(&dir, "pika-network.net", ServerStick::Keys);
        assert_eq!(get(&dir, "pika-network.net"), Some(ServerStick::Keys));
        assert_eq!(get(&dir, "PLAY.example.org:25570"), Some(ServerStick::Keys));
        let _ = std::fs::remove_dir_all(dir);
    }
}
