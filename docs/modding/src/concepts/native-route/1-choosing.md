# Changing the game's own code

Use hooks and Parts when they expose what you need. To change what a game function itself does, take
[the native route](../native-route.md):
tables under `[native]` in `sunrise-mod.toml`, applied to the prebuilt game without rebuilding it.
You work with the game's real types, pinned to the exact game build.

| Table | Use it to | Page |
| --- | --- | --- |
| `[[native.mixin]]` | run code at the start or end of a game function, change a value or a call inside it, or replace its body | [Mixins](2-mixins.md) |
| `[[native.loop]]` | run your code before and after each entry of a hot loop the game publishes, such as the block entity tick | [Published loops](3-loops.md) |
| `[[native.bridge]]` | read, write or call a private field or method of a game type from your mixin code | [Bridges](4-bridges.md) |
| `[[native.module]]` | add a module of your own code inside a game source file, for your mixin code to share | [Support modules](5-support-modules.md) |
| `[[native.state]]` | add a field to a game type: refused, use an attachment | [Native state](6-native-state.md) |
