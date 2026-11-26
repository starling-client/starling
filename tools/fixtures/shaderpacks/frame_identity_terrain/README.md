# Frame identity: terrain

A test fixture of `tools/test_frame_identity_parity.py` (sessions `terrain_pack` and `terrain_pack_hdr`), not a
look and not offered to players: a shader pack in the `shaders/` format (GLSL 120, [shader-packs-spec.md](../../../../docs/cleanroom/shader-packs-spec.md))
whose `gbuffers_terrain` writes a known value, so a change to how terrain reaches a pack program is held to the same
pixels. Every terrain pixel is the colour `(0.25, 0.5, 0.75)` times the lightmap's, dimmed by the LabPBR height in the
normal map's alpha (1 where a block has none) and half transparent for translucent terrain (`renderStage`). The
heights come from a resource pack the harness writes (`relief_pack`).
