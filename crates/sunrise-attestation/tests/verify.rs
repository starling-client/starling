use serde_json::Value;
use sigstore_verify::Error as VerifyError;
use sunrise_attestation::{Attested, Error, PUBLIC_GOOD_TRUSTED_ROOT, verify};

const BUNDLE: &[u8] = include_bytes!("fixtures/ruff-source.sigstore.json");
const DIGEST: [u8; 32] = [
    0x03, 0x78, 0xef, 0x60, 0x35, 0x6c, 0xaa, 0xad, 0x7b, 0x84, 0x65, 0x36, 0x91, 0xa5, 0xd5, 0x7a,
    0x1d, 0x8b, 0x57, 0xfd, 0x59, 0xd1, 0x73, 0xcf, 0xc2, 0x7b, 0x0f, 0xd4, 0xa9, 0x1a, 0x13, 0xf3,
];

#[test]
fn verifies_public_github_provenance_and_returns_certificate_claims() {
    assert_eq!(
        verify(BUNDLE, DIGEST, PUBLIC_GOOD_TRUSTED_ROOT).unwrap(),
        Attested {
            repository_id: 523043277,
            owner_id: 115962839,
            repository_name: "astral-sh/ruff".into(),
            signer_workflow_ref: "astral-sh/ruff/.github/workflows/release.yml@refs/heads/main"
                .into(),
            source_ref: "refs/heads/main".into(),
            source_commit: "a7ba68b4919e56b6af949a42b2da38fe4eec67a9".into(),
            predicate_type: "https://slsa.dev/provenance/v1".into(),
        }
    );
}

#[test]
fn rejects_a_digest_not_in_the_signed_subjects() {
    assert!(matches!(
        verify(BUNDLE, [0; 32], PUBLIC_GOOD_TRUSTED_ROOT),
        Err(Error::Unverified(VerifyError::ArtifactMismatch(_)))
    ));
}

#[test]
fn rejects_a_tampered_dsse_signature() {
    let mut bundle: Value = serde_json::from_slice(BUNDLE).unwrap();
    let sig = &mut bundle["dsseEnvelope"]["signatures"][0]["sig"];
    let mut changed = sig.as_str().unwrap().to_owned();
    let i = changed.len() - 8;
    changed.replace_range(i..i + 1, if &changed[i..i + 1] == "A" { "B" } else { "A" });
    *sig = changed.into();
    let bundle = serde_json::to_vec(&bundle).unwrap();
    assert!(matches!(
        verify(&bundle, DIGEST, PUBLIC_GOOD_TRUSTED_ROOT),
        Err(Error::Unverified(_))
    ));
}

#[test]
fn rejects_a_certificate_outside_the_trusted_root() {
    let mut unrelated_root: Value = serde_json::from_str(PUBLIC_GOOD_TRUSTED_ROOT).unwrap();
    // Keep the authentic log and TSA keys, but swap the current Fulcio CA for
    // the retired CA so the real bundle no longer chains to a trusted anchor.
    unrelated_root["certificateAuthorities"][1]["certChain"] =
        unrelated_root["certificateAuthorities"][0]["certChain"].clone();
    assert!(matches!(
        verify(BUNDLE, DIGEST, &unrelated_root.to_string()),
        Err(Error::Unverified(VerifyError::Certificate(_)))
    ));
}

#[test]
fn rejects_a_tampered_rekor_inclusion_proof() {
    let mut bundle: Value = serde_json::from_slice(BUNDLE).unwrap();
    bundle["verificationMaterial"]["tlogEntries"][0]["inclusionProof"]["hashes"][0] =
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into();
    let bundle = serde_json::to_vec(&bundle).unwrap();
    assert!(matches!(
        verify(&bundle, DIGEST, PUBLIC_GOOD_TRUSTED_ROOT),
        Err(Error::Unverified(VerifyError::TransparencyLog(_)))
    ));
}
