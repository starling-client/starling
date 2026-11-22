//! Opaque identities for host-owned immutable numeric registry snapshots.

/// Identify the retained mod list and, if installed, its immutable Parts epoch.
///
/// These process-local tokens are host plumbing, not a public mod API or wire
/// identity. Installed mod lists are retained for the process lifetime, so their
/// addresses cannot be recycled while a registry refers to them. Parts are named by
/// the epoch each installation was given, which no later installation can reuse, so
/// the identity holds even where a test installation is replaced. Empty
/// mod lists are equivalent regardless of the allocation that supplied them.
/// A registrar still building the next installation has no immutable Parts identity yet.
pub fn registry_installation_identity() -> (usize, Option<usize>) {
    let mods = crate::mods();
    let mods = if mods.is_empty() {
        0
    } else {
        mods.as_ptr() as usize
    };
    let parts = if crate::parts::registration_in_scope() {
        None
    } else {
        crate::parts::installed().map(|parts| parts.epoch as usize)
    };
    (mods, parts)
}
