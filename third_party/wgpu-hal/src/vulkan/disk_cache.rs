//! Horizon: the driver's compiled pipelines kept on disk between runs.
//!
//! Vulkan on Android has no cache of its own: every start compiles Horizon's
//! pipelines again, about 3.5 s of a POCO F6 Pro's (Adreno 740) start and
//! longer on PowerVR. The application names one file with [`set_file`]. Every
//! device opened after that gets a `VkPipelineCache` seeded from the file, and
//! every pipeline made without a cache of its own goes through it
//! (`DeviceShared::fallback_pipeline_cache`). [`save`] writes what the driver
//! holds back to the file.
//!
//! The driver is handed only data it made itself: the file's checksum must
//! match, and the data's own header (`VkPipelineCacheHeaderVersionOne`) must
//! name this GPU (vendor, device and pipeline cache UUID, which changes with
//! the driver). A file that fails either is ignored and written over.
//!
//! File: `MAGIC`, `len: u64 LE`, `check: u64 LE` (FNV-1a 64 of the data), the
//! data. It is written next to itself and renamed over, so a run killed while
//! it writes leaves the last whole file.

use alloc::{
    sync::{Arc, Weak},
    vec::Vec,
};
use std::{fs, io::Write as _, path::PathBuf, sync::OnceLock};

use ash::vk;

const MAGIC: &[u8; 8] = b"HZVKPC1\n";
const HEADER: usize = MAGIC.len() + 8 + 8;
/// Past this the data is not kept: Horizon's pipelines take a few megabytes.
pub const MAX_BYTES: usize = 64 << 20;

static FILE: OnceLock<PathBuf> = OnceLock::new();

/// Keep the driver's compiled pipelines in `path` for the rest of the run.
/// Only the first call counts; without one, devices open with no pipeline
/// cache, as before.
pub fn set_file(path: PathBuf) {
    let _ = FILE.set(path);
}

/// Whether devices get a pipeline cache ([`set_file`] was called).
pub(super) fn enabled() -> bool {
    FILE.get().is_some()
}

/// The data an earlier run left for the GPU `properties` describes, if any.
pub(super) fn seed(properties: &vk::PhysicalDeviceProperties) -> Option<Vec<u8>> {
    let path = FILE.get()?;
    let bytes = fs::read(path).ok()?;
    let data = unwrap(&bytes)?;
    if !made_for(data, properties) {
        log::info!(
            "Vulkan pipeline cache {}: made for another GPU or driver; starting over",
            path.display()
        );
        return None;
    }
    Some(data.to_vec())
}

/// The open devices with a pipeline cache, for [`save`].
static DEVICES: std::sync::Mutex<Vec<Weak<super::DeviceShared>>> =
    std::sync::Mutex::new(Vec::new());

/// A device was opened with a pipeline cache: [`save`] writes what it holds.
pub(super) fn register(shared: &Arc<super::DeviceShared>) {
    if shared.fallback_pipeline_cache == vk::PipelineCache::null() {
        return;
    }
    let mut devices = DEVICES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    devices.retain(|device| device.strong_count() > 0);
    devices.push(Arc::downgrade(shared));
}

/// Write what the newest open device's pipeline cache holds to the file named
/// with [`set_file`]: the driver's data is read here, and the file written on a
/// thread of its own. Nothing without a file or a device.
pub fn save() {
    let shared = DEVICES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .rev()
        .find_map(Weak::upgrade);
    let Some(shared) = shared else { return };
    let data = match unsafe {
        shared
            .raw
            .get_pipeline_cache_data(shared.fallback_pipeline_cache)
    } {
        Ok(data) => data,
        Err(error) => {
            log::warn!("Vulkan pipeline cache: the driver gave no data ({error})");
            return;
        }
    };
    drop(shared);
    let _ = std::thread::Builder::new()
        .name("pipeline-cache".into())
        .spawn(move || {
            if let Err(error) = write(&data) {
                log::warn!("Vulkan pipeline cache: {error}");
            }
        });
}

/// Write `data` (from `vkGetPipelineCacheData`) to the file.
fn write(data: &[u8]) -> std::io::Result<()> {
    // Two saves at once would write the same partial file.
    static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let Some(path) = FILE.get() else {
        return Ok(());
    };
    if data.is_empty() || data.len() > MAX_BYTES {
        return Ok(());
    }
    let _writing = WRITING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let partial = path.with_extension("partial");
    let mut file = fs::File::create(&partial)?;
    file.write_all(&wrap(data))?;
    file.sync_all()?;
    drop(file);
    fs::rename(&partial, path)
}

fn wrap(data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER + data.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(data.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&fnv64(data).to_le_bytes());
    bytes.extend_from_slice(data);
    bytes
}

/// The data inside a file's bytes, when they are whole.
fn unwrap(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.get(..MAGIC.len()) != Some(MAGIC.as_slice()) {
        return None;
    }
    let len = u64::from_le_bytes(bytes.get(8..16)?.try_into().ok()?);
    let check = u64::from_le_bytes(bytes.get(16..24)?.try_into().ok()?);
    let data = bytes.get(HEADER..)?;
    (data.len() as u64 == len && fnv64(data) == check).then_some(data)
}

/// Whether `data` starts with the header the driver of the GPU `properties`
/// describes writes (`VkPipelineCacheHeaderVersionOne`, little-endian on every
/// device this runs on).
fn made_for(data: &[u8], properties: &vk::PhysicalDeviceProperties) -> bool {
    let word = |at: usize| {
        data.get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    word(0).is_some_and(|size| size >= 32)
        && word(4) == Some(vk::PipelineCacheHeaderVersion::ONE.as_raw() as u32)
        && word(8) == Some(properties.vendor_id)
        && word(12) == Some(properties.device_id)
        && data.get(16..32) == Some(properties.pipeline_cache_uuid.as_slice())
}

fn fnv64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu() -> vk::PhysicalDeviceProperties {
        let mut properties = vk::PhysicalDeviceProperties::default();
        properties.vendor_id = 0x5143;
        properties.device_id = 0x4320;
        properties.pipeline_cache_uuid = [7; 16];
        properties
    }

    fn driver_data(properties: &vk::PhysicalDeviceProperties, body: &[u8]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&32u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&properties.vendor_id.to_le_bytes());
        data.extend_from_slice(&properties.device_id.to_le_bytes());
        data.extend_from_slice(&properties.pipeline_cache_uuid);
        data.extend_from_slice(body);
        data
    }

    #[test]
    fn whole_data_for_this_gpu_is_read_back() {
        let data = driver_data(&gpu(), b"compiled pipelines");
        let bytes = wrap(&data);
        let read = unwrap(&bytes).unwrap();
        assert_eq!(read, data.as_slice());
        assert!(made_for(read, &gpu()));
    }

    #[test]
    fn a_short_or_changed_file_is_not_handed_to_the_driver() {
        let data = driver_data(&gpu(), b"compiled pipelines");
        let bytes = wrap(&data);
        assert!(unwrap(&bytes[..bytes.len() - 1]).is_none());
        let mut flipped = bytes.clone();
        let last = flipped.len() - 1;
        flipped[last] ^= 1;
        assert!(unwrap(&flipped).is_none());
        assert!(unwrap(b"not a pipeline cache").is_none());
    }

    #[test]
    fn data_from_another_gpu_or_driver_is_not_handed_to_the_driver() {
        let data = driver_data(&gpu(), b"compiled pipelines");
        let mut other_driver = gpu();
        other_driver.pipeline_cache_uuid = [8; 16];
        assert!(!made_for(&data, &other_driver));
        let mut other_gpu = gpu();
        other_gpu.device_id = 0x4330;
        assert!(!made_for(&data, &other_gpu));
        assert!(!made_for(&data[..20], &gpu()));
    }
}
