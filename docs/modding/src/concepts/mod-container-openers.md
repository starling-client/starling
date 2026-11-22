# Mod container openers

A declared mod block entity with a container menu uses the server's
`ContainerOpenersCounter`. Non-spectator players increment it when their menu
opens, and decrement it when the menu closes, including disconnects. Copper
golems count too. Each block entity owns its own counter. Native menus do not
count simply because they carry a position; a combined container counts both
block entities.

`mod_container_viewers` version 2 reports the previous and current counts
synchronously at these moments. The counter rechecks users in range five ticks
after the first open, then every five ticks while open; these rechecks report
counts even when unchanged. Open and close callbacks can therefore occur in the
same tick. The player is in place during their open or close callback; a recheck
has no acting player. Removing or replacing the block entity drops its pending
recheck without a viewer notice or a game event. A guest's open menu belongs to
that block entity instance, not its position: closing or rechecking the old menu
does not affect a replacement's count or viewer list.

A zero-to-positive transition posts `minecraft:container_open`; a
positive-to-zero transition posts `minecraft:container_close`. The opening or
closing entity is the source. A recheck has no entity source. These game events
reach vibration listeners just as vanilla container events do.

A mod can update its block state or play its own sound from the viewer callback.
Use `previous == 0 && viewers > 0` for an open edge and
`previous > 0 && viewers == 0` for a close edge.
