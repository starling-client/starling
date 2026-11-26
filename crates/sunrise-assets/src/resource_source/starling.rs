//! Starling's part of resource_source.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl ResourceSource {
    /// [`Self::read`], but a folder's file from its disk too when the folder's prepared data
    /// file lacks it: one put there after preparing (Starling's downloaded touch controls),
    /// which `read` reports missing without asking the disk.
    pub fn read_loose(&self, name: &str) -> io::Result<Vec<u8>> {
        match self.0.as_ref() {
            Storage::Directory(root) if stays_inside(name) => match self.read(name) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => fs::read(root.join(name)),
                read => read,
            },
            _ => self.read(name),
        }
    }
}
