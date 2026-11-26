//! Starling's part of index.rs, out of Sunrise's file (tools/starling/move_out.py): what Starling
//! adds there lives here, so a sync with Sunrise meets only the `mod` line.
#![allow(clippy::wildcard_imports)]

#[allow(unused_imports)]
use super::*;

impl Index {
    /// Starling: `other`'s projects and takedowns added to this index's, for a catalog that
    /// lists two registries (Sunrise's and Horizon's). A project both name is this index's.
    pub fn merge(&mut self, other: Index) {
        for project in other.projects {
            if self.project(&project.id).is_none() {
                self.projects.push(project);
            }
        }
        self.revoked.extend(other.revoked);
        self.left_out.extend(other.left_out);
    }
}
