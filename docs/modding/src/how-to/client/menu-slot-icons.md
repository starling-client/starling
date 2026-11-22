# Menu slot icons

In the registered screen factory, declare each visible container slot in `MenuScreenLayout.slots` as `Some(ScreenSlot { position: [x, y], no_item_icon: Some("minecraft:container/slot/shield".into()) })`. Use `no_item_icon: None` for a visible slot with no icon, and a `None` slot entry (or omitted trailing entries) for slots that start inactive/hidden (`Slot.isActive()`). Keep each entry at its physical backing index. Geometry and the optional icon belong to the client screen, including a screen for a bound recipe book.

The icon is vanilla's `Slot.getNoItemIcon`: a GUI atlas sprite identifier, drawn at 16 × 16 when the displayed stack is empty. A quick-craft preview that displays an item suppresses it; a single-slot drag hides the entire slot's contents as usual. Filled slots draw their item instead. Custom backgrounds do not suppress the icon. A local slot move moves the icon with the slot; a local hide suppresses it.

The client looks up icons in the current GUI atlas when drawing and falls back to the missing-texture sprite for unknown identifiers, matching vanilla 26.3. Icon lookup does not allocate during drawing.
