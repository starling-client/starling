//! Load-owned attachment identities. Owner declarations authorize cross-package use.
use super::*;

/// Attachment declarations.
pub trait AttachmentTypesRegistrar {
    /// Declare an explicitly namespaced attachment in a namespace owned by this
    /// package (its manifest namespaces or a content package it serves via `served_by`).
    /// `has_sync_codec` means the type synchronizes to clients, not merely that
    /// it has a persistence codec. At load, declaration conflicts are checked
    /// first, then undeclared uses, then server packages (or `side = "server"`
    /// served namespace suppliers) declaring true.
    /// A server package may use another package's synced attachment.
    /// A refusal fails installation even when the caller ignores this result.
    fn register_attachment_type(&mut self, id: &str, has_sync_codec: bool) -> Result<(), String>;
}

/// Read-only access to the load-owned attachment registry, including without a world host.
// hidden: attachment metadata lookup; authors use typed keys and snapshots.
#[doc(hidden)]
pub trait AttachmentDeclarations {
    fn validate_attachment_type(
        &self,
        namespace: &str,
        name: &str,
        sync: bool,
    ) -> Result<(), crate::AttachmentError>;
}

/// Validate use against the installed owner's declaration.
// hidden: shared engine and host adapter validation, not an authoring method.
#[doc(hidden)]
pub fn validate_attachment_use(
    namespace: &str,
    name: &str,
    sync: bool,
) -> Result<(), crate::AttachmentError> {
    // Engine-owned storage work has no package identity. Every mod dispatch
    // enters its owning package scope before reaching this boundary.
    let Some(package) = crate::content::crash::current_native_mod() else {
        return Ok(());
    };
    validate_declared_attachment(
        installed().map(|parts| &parts.attachment_types),
        package,
        namespace,
        name,
        sync,
    )
}

// Owner policy: the owner's declaration suffices for all callers.
/// Check one id against collected declarations, before or after installation.
// hidden: engine-owned load validation; not an authoring operation.
#[doc(hidden)]
pub fn validate_declared_attachment(
    declarations: Option<&HashMap<String, HashMap<String, bool>>>,
    caller: &str,
    namespace: &str,
    name: &str,
    sync: bool,
) -> Result<(), crate::AttachmentError> {
    let Some(declared_sync) = declarations
        .and_then(|types| types.get(namespace))
        .and_then(|types| types.get(name))
    else {
        return Err(crate::AttachmentError::Undeclared {
            package_id: caller.into(),
            attachment_id: format!("{namespace}:{name}"),
        });
    };
    if sync && !declared_sync {
        return Err(crate::AttachmentError::InvalidSyncPolicy);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owner_declaration_allows_b_to_use_a_mana_and_names_missing_use() {
        let declarations = HashMap::from([("a".into(), HashMap::from([("mana".into(), false)]))]);
        assert_eq!(
            validate_declared_attachment(Some(&declarations), "b", "a", "mana", false),
            Ok(())
        );
        assert_eq!(
            validate_declared_attachment(
                Some(&declarations),
                "test_pkg",
                "test_pkg",
                "mana",
                false
            ),
            Err(crate::AttachmentError::Undeclared {
                package_id: "test_pkg".into(),
                attachment_id: "test_pkg:mana".into()
            })
        );
        assert_eq!(
            validate_declared_attachment(Some(&declarations), "b", "a", "mana", true),
            Err(crate::AttachmentError::InvalidSyncPolicy)
        );
    }
}
