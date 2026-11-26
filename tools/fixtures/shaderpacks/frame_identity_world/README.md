# Frame identity: world

A test fixture of `tools/test_frame_identity_parity.py` (session `world_pack`), not a look and not offered to
players: a shader pack in the `shaders/` format (GLSL 120, [shader-packs-spec.md](../../../../docs/cleanroom/shader-packs-spec.md))
with the programs that draw the world around terrain, so a change to how they reach the pack is held to the same
pixels. Each program changes the picture plainly:

- `gbuffers_skybasic` tints the sky disc and draws the stars cyan and the void dark red, so a sky that falls back to
  the renderer's shows;
- `gbuffers_textured` (the sun and the moon) turns the sun red and the moon green;
- `gbuffers_weather` turns rain and snow orange;
- `gbuffers_terrain` lights terrain with the lightmap and tints translucent terrain blue;
- `composite` grades the whole picture, the End's sky included, which no other program reaches.
