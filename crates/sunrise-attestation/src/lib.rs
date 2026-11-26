//! Offline verification of public GitHub Actions Sigstore artifact attestations.
//!
//! A verified attestation identifies the source and signer; callers still decide
//! which repository, workflow, and predicate to trust for their use.

use serde::Deserialize;
use sigstore_trust_root::TrustedRoot;
use sigstore_types::{Bundle, Sha256Hash, SignatureContent};
use sigstore_verify::VerificationPolicy;

/// Sigstore public-good trust material. Refresh by updating this pinned TUF target.
// https://github.com/sigstore/root-signing/blob/25b222a48cda4aa88e1af85605bdd193f5619a60/targets/trusted_root.json
// SHA-256: 6494e21ea73fa7ee769f85f57d5a3e6a08725eae1e38c755fc3517c9e6bc0b66
pub const PUBLIC_GOOD_TRUSTED_ROOT: &str = include_str!("trusted_root.json");

const GITHUB_ISSUER: &str = "https://token.actions.githubusercontent.com";
const GITHUB_URL: &str = "https://github.com/";

/// Claims from the verified signing certificate and DSSE payload.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attested {
    /// Stable GitHub repository identifier, not its name or URL (Fulcio .1.15).
    pub repository_id: u64,
    /// Stable GitHub identifier of the source repository's owner account (Fulcio .1.17).
    pub owner_id: u64,
    /// Source repository in `owner/name` form (Fulcio .1.12).
    pub repository_name: String,
    /// Builder in `owner/name/.github/workflows/file.yml@ref` form (Fulcio .1.9).
    /// This is the called workflow for a reusable workflow, not its caller.
    pub signer_workflow_ref: String,
    /// Git ref of the source repository (Fulcio .1.14).
    pub source_ref: String,
    /// Source commit (Fulcio .1.13), not the reusable workflow's commit.
    pub source_commit: String,
    /// Type of the signed in-toto predicate.
    pub predicate_type: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid Sigstore bundle JSON: {0}")]
    Bundle(#[source] serde_json::Error),
    #[error("expected a DSSE attestation bundle")]
    NotDsse,
    #[error("invalid Sigstore trusted root JSON: {0}")]
    TrustedRoot(#[source] sigstore_trust_root::Error),
    #[error("attestation verification failed: {0}")]
    Unverified(#[from] sigstore_verify::Error),
    #[error("attestation certificate has missing or invalid {0}")]
    Claim(&'static str),
    #[error("invalid in-toto predicate type: {0}")]
    Predicate(#[source] serde_json::Error),
}

/// Verify a keyless DSSE bundle against a precomputed SHA-256 and trusted root JSON.
///
/// Verifies the Fulcio chain at an authenticated signing time, its certificate
/// transparency proof, Rekor inclusion, and the DSSE signature. The signed
/// in-toto statement must name this SHA-256 as a subject digest. No network or
/// system trust store is consulted. `trusted_root` must come from a trusted
/// source, such as [`PUBLIC_GOOD_TRUSTED_ROOT`], not from the artifact publisher.
pub fn verify(
    bundle_json: &[u8],
    artifact_sha256: [u8; 32],
    trusted_root: &str,
) -> Result<Attested, Error> {
    let bundle: Bundle = serde_json::from_slice(bundle_json).map_err(Error::Bundle)?;
    let SignatureContent::DsseEnvelope(envelope) = &bundle.content else {
        return Err(Error::NotDsse);
    };
    let trusted_root = TrustedRoot::from_json(trusted_root).map_err(Error::TrustedRoot)?;
    let policy = VerificationPolicy::any_identity().require_issuer(GITHUB_ISSUER);
    let verified = sigstore_verify::verify(
        Sha256Hash::new(artifact_sha256),
        &bundle,
        &policy,
        &trusted_root,
    )?;
    let claims = &verified
        .certificate()
        .ok_or(Error::Claim("signing certificate"))?
        .ci_claims;
    let claim = |value: &Option<String>, label| {
        value
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or(Error::Claim(label))
    };
    let github_claim = |value: &Option<String>, label| {
        value
            .as_deref()
            .and_then(|s| s.strip_prefix(GITHUB_URL))
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or(Error::Claim(label))
    };
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PredicateType {
        predicate_type: String,
    }
    let PredicateType { predicate_type } =
        serde_json::from_slice(envelope.payload.as_bytes()).map_err(Error::Predicate)?;
    Ok(Attested {
        repository_id: claims
            .source_repository_identifier
            .as_deref()
            .and_then(|s| s.parse().ok())
            .ok_or(Error::Claim("source repository ID"))?,
        owner_id: claims
            .source_repository_owner_identifier
            .as_deref()
            .and_then(|s| s.parse().ok())
            .ok_or(Error::Claim("source repository owner ID"))?,
        repository_name: github_claim(&claims.source_repository_uri, "source repository URI")?,
        signer_workflow_ref: github_claim(&claims.build_signer_uri, "build signer URI")?,
        source_ref: claim(&claims.source_repository_ref, "source ref")?,
        source_commit: claim(&claims.source_repository_digest, "source commit")?,
        predicate_type,
    })
}
