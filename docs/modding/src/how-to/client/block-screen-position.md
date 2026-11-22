# Reading the level from a block screen

In a menu screen factory, `open.pos` is the position for this opening and `open.block` is
the verified mod block ID, if there is one. Capture the position by value with
`MenuScreen::with_draw(move |host, frame, draw| ...)`. Decode one-time payloads
from `open.extra` in the factory; once the window is shown those bytes are no
longer available. Live values arrive separately in `frame.data`. Without a
matching server payload, only a verified clicked opener supplies a position. A menu
explicitly opened without a position reports `None`, not a fabricated origin.

Use this position with the existing both-sided level reads through the client host to inspect the client level view, for example the block beneath a machine. This view may lag the server and is not a substitute for authoritative menu data. The draw callback remains read-only and fires beneath items and labels.
