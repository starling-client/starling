
## Entity snapshot revision boundary

`client_update_fields` is on the wire but outside the entity revision counter (`containers.rs:18`, `lib.rs:312`, `containers.rs:75`). Any future revision-keyed cache must treat it as excluded from the revision guarantee and synchronize it independently. Stage 1b must clarify the `ModRevision` documentation's "Equal revisions, equal contents" statement accordingly.

## Provider-query review: trust boundaries and native-side notes

The implementation at fdccc11a1d and exact-variant tests at 40b17368f1 were independently reviewed. Full structural reply decoding, including trailing-byte validation and static owner conversion, completes before any cursor writeback. The reviewer reproduced refusal without cursor mutation for incomplete cursors, invalid Option/bool tags and trailing bytes, and killed missing-writeback and late-validation mutants.

The stale provider-id and backwards / u32::MAX cursor probes forge the trusted HOST reply after the native encoder has returned. They do not establish hostile GUEST control of production provider-query replies. Native HostCodec creates ordinary Held ids by inserting their objects into its live generational table; return-to-origin erased values are resolved against the guest's typed handler table. Native invocation and release check incoming native Held ids. No HOST_VALIDATE request or additional per-resource round trip was added.

Cursor writeback remains structural u32 writeback, not a semantic table-bound proof. The current reply carries no native table length. Enforcing that upper bound needs new source-owned receipt machinery; classifying arbitrary mutable u32s as monotone cursors would change the generic scalar contract. That hardening was explicitly excluded from this change. Production InstalledProviders bounds its input cursor with native table access; the review found no guest-controlled override of that installed table. Do not interpret the host-forgery probes as a wasm coverage gap or as proof that arbitrary cursor values satisfy native enumeration semantics.

Native-side note, not a wasm gap: a buggy custom native CapabilityProviders implementation may return Some without advancing its cursor, causing the existing native ask_providers consumer to repeat indefinitely. The progress-check prototype e12d95289a is deliberately not merged, following the narrowed review scope. It needs a fresh review before any future integration; its once-only logging was not verified here.

Real Wasmtime group/provider behavior and budget exhaustion are separate integration checks, not established by the native loopback or forged-host transport probes.
