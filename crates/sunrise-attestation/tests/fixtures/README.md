# Public GitHub Actions provenance fixture

`ruff-source.sigstore.json` is the `bundle` with `initiator: "user"` from
[GitHub's attestation API for astral-sh/ruff](https://api.github.com/repos/astral-sh/ruff/attestations/sha256:0378ef60356caaad7b84653691a5d57a1d8b57fd59d173cfc27b0fd4a91a13f3).
It attests `source.tar.gz` from [Ruff 0.17.6](https://github.com/astral-sh/ruff/releases/tag/0.17.6).
The artifact itself is not included. Its SHA-256 (from the release asset metadata) is
`0378ef60356caaad7b84653691a5d57a1d8b57fd59d173cfc27b0fd4a91a13f3`.

The fixture has a public-good Fulcio certificate and a Rekor DSSE entry with a
signed inclusion proof and SET. It is checked offline with the bundled trust root.
The upstream project's [MIT license](LICENSE-MIT) is included.
