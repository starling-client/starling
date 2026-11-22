# Level container and Part state reads

`LevelHost` adds block entity lookup, side-local container size and slot snapshots, and read-only BlockEntityPartState reads. These never load chunks or unpack loot. Container snapshots refuse clients; clients retain only local block entity handles and no server Part state.

`analog_output` version 2 now receives `&mut dyn LevelRandomHost` instead of `&dyn LevelHost`. Its `container_signal` runs the existing engine comparator path, unpacking unopened loot once before calculating a stack-limit-aware signal. It refuses clients and unloaded positions and never loads chunks. The host change preserves vanilla's lazy loot generation and random draw ordering without interior mutation.
