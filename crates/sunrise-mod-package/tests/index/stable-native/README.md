# Stable native release fixture

`source.smod` pins `26.3-0.1.0`. `valid.binaries.json` describes a rebuild for `26.3-0.1.1`, with the Linux client and dedicated server host names as SDK metadata names them. The archives contain a real stripped x86-64 ELF shared library (`cc -shared -fPIC -nostdlib -Wl,--build-id=none -s` on a C function that returns a constant). It has no Sunrise entry point and is never loaded. The other signed descriptors omit a required host, substitute the server archive for the client, or carry a signature from a different key. The source variants exercise the lockfile requirement and the builder's Cargo feature refusal.

`schema-1.json` shows the same archive records as the registry emits, alongside an unrelated source-only project and a project that depends on the native mod's source version. It is also used to test the unmodified schema-1 reader.

`wrong-architecture/` signs the same source and a client archive with a real i386 shared library (`cc -m32`), while the filename and stamp still claim x86-64. Its server archive reuses the healthy archive from this directory, signed with the test's separate key. The same key signs `short-label.binaries.json`, whose archive names are canonical but whose descriptor version is abbreviated.

`author-publish/` signs a source with a valid uses list for the new-source lint, plus the original archives and a descriptor, with another disposable key.

These packages and tar.xz archives were assembled independently of `cargo sunrise-mod`. The `.minisig` files and embedded archive signatures were made with disposable fixture keys; only the project public key is retained.
