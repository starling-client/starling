# Keep the bound chunk generator identifier (#6115)

`handle:LevelRef::generator_type` reports the generator the server actually built. A flat stem
can take precedence over the saved Debug preset. The handle's signature is unchanged.

The engine retains the validated identifier from registration: `ChunkGeneratorInfo.id` and
`mod_api::__private::chunk_generator_declarations` now carry `Identifier` instead of text.
These are hidden integration items, not mod-facing catalog entries. Mods need no source changes;
runtime generator requests and dimension settings still use the same fully qualified strings.
