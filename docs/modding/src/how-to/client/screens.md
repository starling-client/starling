# Screens, the inventory key and the sign editor

A client mod can close the screen a player has open, open the player's inventory, and read which sign
the sign editor is on. The two writes take the same paths as the keys the player uses, so the server
sees only the packets of a normal client.

```rust
// Escape: close whatever is open.
if host.close_screen() {
    // a screen was open and is now closed
}

// The inventory key, key.inventory.
if host.open_inventory() {
    // the inventory screen is up (or the ridden vehicle's screen was asked for)
}

// Which sign the editor is on, if any.
if let Some(sign) = host.sign_editor() {
    let _ = (sign.pos, sign.front);
}
```

`ClientHost::close_screen()` follows the screen's own Escape close. A container screen closes as the
player's container close: the client sends `ServerboundContainerClosePacket` with the menu's id and then
closes its copy of the menu. It never drops a container menu locally alone, which would leave the
server's menu open. A pause menu closes to the game, another screen runs its own close, and the sign
editor sends `ServerboundSignUpdatePacket` with the lines being edited, as Escape does. A written book
or a book and quill closes and sends nothing: no menu close, and no `ServerboundEditBookPacket`, since
Escape saves no pages (only the editor's Done button does). A lectern's book closes the lectern's menu,
so the client sends `ServerboundContainerClosePacket` with that menu's id.

When a screen opens over another, only the screen in front handles the close. A link's or a command's
confirmation answers no, a server's resource pack prompt declines, and a dialog runs its own Escape.
Declining a required pack disconnects instead of returning to the screen underneath. The death
screen's exit confirmation answers no once its Escape delay has elapsed. The death screen itself,
the code of conduct and a dialog that Escape cannot close do not close this way. The screen hooks'
closing notices fire as they do for Escape. It returns `false` when no screen is open or the screen
in front does not close.

`ClientHost::open_inventory()` does what `key.inventory` does: it opens the player's inventory screen
locally and sends nothing, because the player's menu is always open on the server. Riding a vehicle
with its own inventory (a tamed mount or a chest boat), it sends the open-inventory player command
(`ServerboundPlayerCommandPacket`) at the player's next tick instead, as the key's press is. It returns
`false` with a screen already open, for a spectator, in a portal transition, or with no world.

`ClientHost::sign_editor()` returns `Some(SignEditView { pos, front })` while the sign editor is open:
the sign's block position and whether its front side is being edited. It returns `None` with no sign
editor open, or outside a world. A close of the sign editor always sends the sign update, however it
closes; there is no close that skips it, because the vanilla client never omits that packet.

## World preset editors

Bind a `PresetEditor` to a world preset with `register_preset_editors`. The stock frame draws first,
then `PresetEditor::render` adds the editor's paint, followed by `screen_render_post` and toasts.
If the render callback fails, its paint and draft edits are discarded without removing the stock
frame or skipping Post. A panic disables the bound editor callbacks for the session and reports the
mod's name in the log and an error toast.

While `world_preset_editor` is open, keys consumed by the input method during composition do not
reach `PresetEditor::key_pressed` or the editor's Done and Cancel buttons. Committed characters go
through the usual character hooks and `PresetEditor::char_typed`, separately from key presses.

The editor's Done and Cancel buttons take a left click before `PresetEditor::mouse_clicked(x, y,
button, double_click, draft)`. The editor receives the same `i32` GLFW button index as the screen
hooks. Press Pre, the editor, and press Post get the same `double_click`: the last handled press
must have been on the same screen instance with the same button, less than 250 ms earlier.
Synthetic clicks pass `false` and do not change the history.

If Pre allows a press or release, its button Post runs whether the original screen took or declined
that event. Done and Cancel report `world_preset_editor` with `event.was_click_handled = true`, even
when Create World is already open. A release instead reports its own receiving screen and
`event.was_release_handled`. Post listeners may write `*event.result = TriState::True` to force
handled or `*event.result = TriState::False` to force unhandled; `TriState::Default` keeps the
original answer. The final answer decides whether to record a press for double-click detection and
whether the event can reach gameplay mappings with no screen open. A cancelled mouse Pre reaches
neither the editor nor Post, but a cancelled real press counts as handled. Key and character Post
still run only for unhandled input.

The control socket sends `screen_key_released_pre` to the currently open title screen, even if
that screen did not receive the matching press. For example, when Escape closes the preset editor,
its release goes to Create World, including when Escape is sent as a single tap. The released key
is no longer held. If it was a modifier, its bit is already cleared in `event.modifiers` unless
the other key for the same modifier is still held.

## Drawing a dimension transition

In `Hooks::register_dimension_transition_screen`, push a `DimensionTransitionScreen` for
each trip you want to cover. Each mod receives an empty batch at client startup.
`Conditional { to, from }` wins over `Incoming(to)`, then `Outgoing(from)`. The first valid
registration for each key wins.

```rust,ignore
use mod_api::{
    ClientHost, DimensionTransition, DimensionTransitionReason, DimensionTransitionScreen,
    DrawContext, RegisterDimensionTransitionScreenEvent, RegistrationContext,
};

fn register_dimension_transition_screen(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterDimensionTransitionScreenEvent<'_>,
) {
    event.screens.push(DimensionTransitionScreen::new(
        DimensionTransition::Incoming("example:moon".into()),
        |_, initial_reason| {
            // Chosen once when the screen opens, not when the tracker restarts.
            let backdrop = match initial_reason {
                DimensionTransitionReason::NetherPortal => 0xFF301010,
                _ => 0xFF101030,
            };
            Box::new(
                move |_: &dyn ClientHost, _reason, progress, draw: &mut dyn DrawContext| {
                    let [w, h] = draw.size();
                    draw.fill([0.0, 0.0, w, h], backdrop);
                    if let Some(progress) = progress {
                        draw.fill([0.0, h - 8.0, w * progress, 8.0], 0xFF6060FF);
                    }
                },
            )
        },
    ));
}
```

The client calls the matching factory once when it opens a new loading screen for travel.
The returned `Box<dyn DimensionTransitionPresentation>` can hold a closure, as above, or an
object implementing the trait. It draws the entire screen under its registering mod's identity.
Its `reason` and optional smoothed integrated-server `progress` are current each frame.
If another load starts while the screen is open, the same presentation stays in place,
even if the reason changes. The tracker still decides when the screen closes. Login and death
respawn do not look up dimension keys; like unmatched travel, they use vanilla when opening
a new loading screen. The vanilla screen also draws when the matching factory or its presentation
is disabled or refused. A failed draw leaves neither its paint nor its requests for blur, local sound,
particle clears or camera changes. Requests pending before that draw stay in place. A phase refusal only
affects that invocation. A draw panic disables drawing for that registration for the session, but does
not stop the factory from being called on the next matching load. The presentation is dropped as its
registering mod when the screen closes or the client disconnects, even if drawing has already been
disabled. A destructor panic is reported with the mod's name and disables the registration's draw
callback, not the factory. Closing still completes.

## Reading open trades

From a client hook in the world, get the menu from `host.local_player()` and
`PlayerRef::container_menu(host)`, then call `menu.merchant_offers(host)`. The result is an owned
snapshot of the latest offers the server sent to the open merchant menu, in display order. Each
`MerchantOffer` has the first payment, an optional second payment, the result stack and its
components, and stock and pricing data. Use `offer.cost_a_count(host)` for the adjusted first
payment and `offer.out_of_stock()` to check availability.

On a Sunrise server, an edit accepted by `trade_with_villager` reaches this snapshot when the
whole result click commits. A listener that only observes a trade does not move the ordinary
server offer refresh earlier. A take is stale if a listener wrote through the `Host` to a player's
inventory, cursor or menu storage (the take read its own player's; a write to any player's counts).
A refused click sends the authoritative offers and contents again to correct the client's prediction.

`Ok(None)` means the handle is no longer current, doesn't belong to the local player, or isn't a
merchant menu. An open merchant menu can return `Ok(Some(vec![]))` before the offers arrive.
`Err(LevelReadError::Unavailable)` means the snapshot couldn't be read. The call doesn't select
an offer, buy anything, or send packets; the server still decides every trade.

## Frame coordinates

A [registered menu screen factory](../world/machine-menu-data.md#the-menus-screen)
gives one opening its own callbacks. Use `with_draw(|host, frame, draw| ...)`,
`with_click(|host, frame, right| ...)`, or
`with_tooltip(|host, frame, slot, lines| ...)` on the returned `MenuScreen`.
Each callback receives a `MenuScreenFrame`. Its `pointer: [i32; 2]` and `gui_size: [i32; 2]`
are the current absolute pointer and window size in scaled GUI pixels. The absolute
pointer truncates toward zero, including outside the panel. `origin: [f32; 2]`
is the panel's top-left; `at: [f32; 2]` is the pointer relative to that origin,
without truncation. Use these values for hover regions and layout after a resize
or panel move. `frame.data` borrows the current menu's live data slots, which may
not have arrived yet. Use `frame.data.get(index)`: a block-backed menu sends
signed-short values at indices 0 through 32,767, and the negotiated blockless
revision-2 payload sends `i32` values at indices 0 through 65,535, both in snapshots and in deltas.
Higher indices stay on the server. Live data is not the opening payload: decode `open.extra` in the
factory and capture only the owned values the callbacks need.

To draw a sprite from synchronized menu data, see
[Draw a progress bar and a fuel gauge](menu-gauges.md).

## What a screen draws beside its panel

`ScreenView::rect` is a container screen's panel (vanilla's `leftPos`, `topPos`, `imageWidth`, `imageHeight`).
`ScreenView::regions` lists what the same screen drew outside it. Each region is a `ScreenRegionKind`
and a `rect`, `[x, y, width, height]` in GUI pixels from the window's top left, taken from the layout that draws it:

| Kind | Present while | Rectangle (vanilla 26.3) |
|---|---|---|
| `RecipeBook` | the recipe book is open beside the window (a window too narrow draws it over the container instead, and it isn't listed then) | its panel, 147 x 166, left of the container's |
| `RecipeBookTabs` | the recipe book drew tabs | the box of the tabs it drew at `xPosTab` (30 pixels left of the book), 35 x 27 a tab, 27 apart, the selected one 2 further left, an animating one as squeezed; a registered book's tab arrows too |
| `CreativeTab` | the creative inventory is open | one per tab of the page shown, 26 x 32, as drawn by `CreativeModeInventoryScreen.extractTabButton` above and below the panel |
| `Effects` | effects show beside the inventory | the effects' box, compact or full, after `render_inventory_mob_effects` moved or resized it |

A region that's hidden, folded or too narrow to show isn't in the list, and a screen with none has an empty list.
The list is what was drawn: `screen_render_post` sees this frame's drawing (none when `screen_render_pre` cancelled
it). Render Pre, Background and Foreground, like input hooks, see the last published draw for that screen.
The hooks that come before a screen's layout (`screen_opening`,
`screen_closing`, `screen_init_post`, `screen_room`, `render_inventory_mob_effects`) see an empty list.
A mod that lays out its own panel beside a screen (a recipe viewer's item list, an overlay) keeps clear of
`rect` and of every region:

```rust,ignore
fn screen_render_post(&self, _host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
    let screen = event.screen;
    let overlaps = |a: [f32; 4], b: [f32; 4]| {
        a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
    };
    let taken = |cell: [f32; 4]| overlaps(screen.rect, cell) || screen.regions.iter().any(|r| overlaps(r.rect, cell));
    // ... lay out the panel's cells, skipping each cell for which `taken(cell)` holds.
}
```

The list is collected after the screen draws, and only while a mod listens to a hook that is given the screen's `ScreenView`, and
it is borrowed, so reading it costs nothing. It holds the game's own regions only. What another mod's screen draws beside its window (an upgrade
column, a row of tabs) is that mod's to report: on NeoForge that's JEI's `IGuiContainerHandler.getGuiExtraAreas`, and
here the viewer mod's own library API.

## Replacing a mod container slot's tooltip

A screen callback installed with `MenuScreen::with_tooltip` runs for a hovered filled slot as well
as an empty slot or screen space. `slot` is the **menu index**, including
the player inventory, or `None` outside a mapped slot. It is not the engine's
internal slot encoding. `frame.at` gives GUI pixels from the screen's origin.

On a filled slot, `lines` starts as the item's tooltip after item tooltip
hooks. Edit, replace or clear those lines: the final list replaces the screen's
`getTooltipFromContainerItem` result before the stack's tooltip image and
`RenderTooltipEvent.GatherComponents`. Empty slots and screen space start with
an empty list. Lines are `(Text, u32)` pairs, not strings: preserve their rich
components and colors when adding a hint or editing another line.

The stack's tooltip image and style, and the rule hiding slot tooltips while
carrying an item (except items such as bundles), are unchanged. This is the
screen-side replacement point, not a second item-tooltip hook. NeoForge's
tooltip events do not carry a slot index; the index here is a Sunrise convenience
over the screen override.

## Moving or hiding a mod screen's slots

The [menu screen factory](../world/machine-menu-data.md#the-menus-screen) puts geometry in
`MenuScreenLayout::slots` at the original backing indices. Only
[permanent exclusions](../world/machine-menu-data.md#custom-menu-screens-and-initially-inactive-slots)
may be `None` or omitted from the end of the list. A layout that changes while open (tabs,
a collapsing panel or a scrolling slot grid) moves or hides slots afterwards with
`ClientHost::set_screen_slot`, as a Java screen sets `Slot.x`/`Slot.y` or deactivates a slot:

```rust,ignore
const MENU: &str = "mymod:station";
// A scrolling grid: slot 3 now sits one row up, 18 pixels from the screen's top-left corner.
host.set_screen_slot(MENU, 3, Some([62.0, 18.0]))?;
// This layout doesn't use slot 4.
host.set_screen_slot(MENU, 4, None)?;
// Show it again on another tab with set_screen_slot(MENU, 4, Some([x, y])).
// Where slot 3 is from the next frame.
assert_eq!(host.screen_slot(MENU, 3), Some([62.0, 18.0]));
```

`menu` is the menu type whose screen is open, so a call made for another screen refuses. Any mod can
read, move, activate, or hide slots of the open screen for `menu`. `slot` is the index into
`MenuScreenLayout::slots` (including slots declared `None` at open, which start inactive and can be
activated later with `Some([x, y])`). A position is GUI pixels from the screen's top-left corner and names the item's top-left
corner, as in `MenuScreenLayout::slots`; `None` hides the slot. The call takes `&self`, so a screen hook such as
`screen_render_post` can move slots every frame; a call that repeats a slot's current place changes nothing and
costs no layout work. The change lasts for that opening. A replacement starts from its declared layout,
even when the server reuses the same window id without a close packet. `ClientHost::screen_slot(menu, slot)` reads where a slot will be from the next frame, `None` when it is
hidden.

From the next frame a moved slot is a vanilla slot at that position, which is what the screen reads live:

- the hovered slot is the first shown slot in menu order whose 18 by 18 box (one pixel up and left of the item
  square, one beyond it) holds the pointer, so a container slot over the player's inventory or hotbar wins;
- the 24 by 24 highlight is drawn 4 pixels up and left of it;
- its item, count and decorations, and its empty-slot icon, are drawn there;
- the recipe book's ghost items are drawn there.

A hidden slot is none of these: it is not hovered, drawn or listed until the mod puts it somewhere again. A move
or a hide never touches the recipe book: a bound book keeps its catalogue and its placement. The recipe book and
`screen_room` still move the whole image, and every slot with it; a slot's own position never changes for them. A
position outside the image is allowed and is not clamped, and a slot there takes the click instead of making it a
click outside the screen.

The call is client-side only: nothing is sent, and server-side slot rules never change. A locally hidden
slot keeps its item and a shift-click may still fill it, as a deactivated Java slot (`Slot.isActive()`)
does. Slots declared `None` at open start inactive and can be activated later with `Some([x, y])`.

Each refusal changes nothing and is named by `ScreenSlotRefusal`:

| Refusal | When |
|---|---|
| `NoScreen` | No mod screen is shown, or the shown one belongs to another menu type |
| `NoSuchSlot` | `slot` is outside the menu's container backing |
| `BadPosition` | An `x` or `y` that is NaN or infinite |
| `Unavailable` | No game backend: screens are the client's |

It moves only the slots a mod screen declares; the player's inventory stays under `inventory_x` and
`inventory_y`.

## Mod container title and inventory labels

`MenuScreenLayout::title_label` and `MenuScreenLayout::inventory_label` optionally place those two labels,
independently of slot positions. `Some([x, y])` is relative to the screen image's
top-left corner, in GUI pixels. `None` uses vanilla's defaults: title `[8, 6]` and
inventory `[8, height - 94]`. Moving `inventory_x` or `inventory_y` moves slots,
not labels. `own_labels: true` suppresses both engine labels even when positions
are supplied; draw them in the callback you install with `MenuScreen::with_draw`.

The default inventory label is at y 72 for a standard 166-high screen and y 39
for a 133-high hopper layout. Chest layouts use `height - 94`; a shulker box's
167-high image places it at y 73.
When building a complete `MenuScreenLayout` literal, include `title_label: None` and
`inventory_label: None`, or use `..Default::default()`.

## Buttons with your own art

Set `Button::sprites` before adding a button to your `Widgets`. With `None` (the default), the
button uses the vanilla sprites. With `Some`, it uses the same GUI sprite names as `DrawContext::sprite`:

```rust
use mod_api::client::widgets::{Button, ButtonSprites, Widgets};

let mut widgets = Widgets::new();
let mut art = ButtonSprites::new("mymod:open");
art.highlighted = Some("mymod:open_highlighted".into());
art.disabled = Some("mymod:open_disabled".into());
art.label_offset = [1.0, -1.0];
let mut button = Button::new([20.0, 24.0, 80.0, 20.0], label);
button.sprites = Some(art);
let open = widgets.add_button(button);
```

`Widgets::draw` stretches the full sprite into the button's rectangle; it doesn't nine-slice custom
art. Inactive takes precedence over hover or focus. An omitted or unknown state sprite falls back to the
custom normal sprite, then to the vanilla nine-slice for that state. The label offset is in GUI pixels;
it moves the text, not the hitbox or the scrolling label's clip. The same button still registers for
narration and controller focus, and `Widgets` handles clicks, Tab, Enter and Space as before.
If you call `DrawContext::widget_button` directly, pass the art as `Option<&ButtonSprites>` (`None` for
vanilla art).

## Labels: wrapped text without a widget

A label is text laid out in a box: measured, wrapped and placed, and nothing else. Call
`DrawContext::text_lines` from a drawing hook with the component, the box's width, an alignment and the
scale the label is drawn at, and draw the lines it answers with where it says:

```rust
// From a menu screen draw callback, gui_render_post or another drawing hook.
let scale = 2.0;
let lines = draw.text_lines(&label, 120.0, TextAlign::Centre, scale)?;
let mut draws = Vec::with_capacity(lines.lines.len() + 2);
draws.push(GuiDraw::Push(vec![PoseOp::Scale(scale, scale)]));
for line in &lines.lines {
    draws.push(GuiDraw::Text {
        text: line.text.clone(),
        x: (x + line.at[0] / scale) as i32,
        y: (y + line.at[1] / scale) as i32,
        color: 0xFFFF_FFFF,
        shadow: false,
    });
}
draws.push(GuiDraw::Pop);
draw.gui_paint(&draws)?;
// lines.width and lines.height size the box you reserved for it.
```

The layout is measured unscaled and answered scaled: the text wraps at `120 / scale` GUI pixels, and
every dimension and position `text_lines` answers — each line's `at` and `width`, the block's `width`
and `height` — is in scaled GUI pixels, which is why the lines are drawn inside the same
`PoseOp::Scale`. The positions are already post-scale coordinates, whereas coordinates inside the
scale pose are unscaled, so the example divides each line's `at` by `scale` before drawing. A `scale` of one is the text at its own size.

Wrapping is vanilla's: a line breaks at a space that fits, a single word wider than the box is broken at
a character rather than dropped, and an explicit `\n` starts a new line, so a trailing `\n` leaves an
empty last line. A width below one pixel still wraps, at one pixel, as `Font.split` does: too narrow a
box gives one character a line rather than none, and one character alone can still make a line wider
than the box. Leading spaces are characters like any other and stay in the line.

Each line keeps its styles, and its legacy `§` codes are applied to those styles and dropped, so a line
drawn alone draws exactly what it measured: a `§r` goes back to the style its run carried, so right
after a break it restores the tail run's carried style (a bold run stays bold). Draw the line through `GuiDraw::Text`; `DrawContext::text` draws plain
characters only. A centred line sits at `wrapping width / 2 - floor(line width / 2)`, as vanilla's
`TextAlignment.CENTER.calculateLeft` floors the half line width. `height` is the last line's `at[1]`
plus one line height, so a scroll container sizes its content from it, and an empty component has no
line at all.

It refuses, answering no line at all, a `TextLinesRefusal` naming why: a width that is not finite (or
that dividing by `scale` overflows), a `scale` that is not finite or is zero or less, text that wraps
into more than 512 lines, or lines holding more than 8192 characters together. This is in-process
layout, so the serialization depth limit does not apply. Inline sprites and player heads remain
objects in the returned lines, with their fallbacks.

## An item or a model turned in 3D

`DrawContext::model_3d(subject, view)` draws one item or one resolved standalone model in a
screen under a turn of the mod's own: a symmetry wand's screen shows its wand turned about all
three axes and its mirror under a pose it built itself.

```rust,ignore
// 30 degrees about the viewer's axis, as a row-major 3x4 matrix.
let (s, c) = 30f32.to_radians().sin_cos();
let turn = [c, -s, 0.0, 0.0, s, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
draw.model_3d(Gui3dSubject::Item(&wand), Gui3dView::new([60.0, 80.0], 64.0, turn))?;
draw.model_3d(Gui3dSubject::Model(self.mirror), Gui3dView::new([120.0, 60.0], 40.0, IDENTITY_TRANSFORM))?;
```

- `Gui3dSubject::Item(&stack)` borrows the stack for the call, leaving it available for another
  draw. The item looks as it does in a slot, without its count, damage bar or cooldown: its model's
  `gui` display transform comes first, then `view.transform`, with the GUI light the model asks
  for. `Gui3dSubject::Model(model)` still takes a handle from `RenderAssets::model` by value. Its
  0 to 1 cube is drawn about its centre, full bright and shaded by face as a scene's blocks are.
- `Gui3dView { at, size, transform }`: `at` is where the subject's centre goes, in GUI pixels,
  and `size` the pixels per block (an item is a slot's size when `size` is 16). `transform` is a
  row-major 3x4 affine matrix applied about the centre in the frame `x` right, `y` up, `z`
  towards the viewer, so a rotation about any axis, roll included, needs no agreed Euler order.
  Animate by passing a new matrix each frame.
- The view is orthographic. Both subjects have a depth of their own: a draw never intersects
  what the screen drew before or after it, and later drawing covers it in call order. An item's
  faces meet each other by depth, as a model's do, and its enchantment glint follows them. The
  OptiFine CIT glint layers of a stack are not drawn on this path yet
  ([#3555](https://github.com/sunrise-org/sunrise/issues/3555)). There is no z input. `push_clip` cuts both
  as it cuts `fill`.
- In an available draw context, a `minecraft:air` stack is a no-op (`Ok(())`) even with an
  invalid view or unreadable component overrides, matching `GuiGraphicsExtractor.item`. Refusals draw nothing: `Gui3dError::Unavailable` (no game, or a
  server), `NotFinite` (a non-finite view value or transformed corner, or a non-positive size),
  `UnknownItem` (an unknown key or an item missing from the current registry), `InvalidStack`
  (component overrides that cannot be converted for drawing), and `Model(ModelError)` (a stale
  or unknown handle).
