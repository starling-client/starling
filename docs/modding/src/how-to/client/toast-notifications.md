# Showing and filtering toast notifications

## Show a notification

Call `ClientHost::add_toast` from a client callback. The client draws it with
its other toasts; you do not need a GUI layer or a render callback.

```rust,ignore
use mod_api::{AddToastError, ClientHost, Identifier, ModToast, Text, ToastIcon};

fn show_notice(host: &dyn ClientHost) -> Result<(), AddToastError> {
    let mut toast = ModToast::new(Text::literal("Finished"));
    toast.description = Some(Text::literal("Your order is ready."));
    toast.icon = Some(ToastIcon::Sprite(
        Identifier::parse("minecraft:icon/checkmark").unwrap(),
    ));
    toast.kind = Some(Identifier::parse("example:order_status").unwrap());
    host.add_toast(toast)
}
```

Use `ToastIcon::Item` for an item stack instead of a GUI-atlas sprite. Both
the icon and description are optional. Invalid text or an invalid item stack
returns `AddToastError` without queueing a toast. A namespaced kind replaces the visible
or queued mod toast with that kind, in place; leave it unset to always add a
separate toast. `display_ms` defaults to vanilla `SystemToastId`'s 5000 ms
and is multiplied by the player's Notification Time setting.

Submission is deferred until the next toast update. Each request passes through
`toast_add` before it changes the queue. Cancelling a replacement leaves the old
toast untouched. A request made inside `toast_add` is offered at the following
update, not during that callback.

## Filter notifications

Implement `Hooks::toast_add` to filter a client notification before it enters its
queue (NeoForge `ToastAddEvent`). The hook receives a read-only `ClientHost` and
`ToastAddEvent`: `kind` is a `ToastKind`, and `title` and `description` are mutable
text components. Return `Flow::Cancel` to discard only that candidate, or
`Flow::Continue` after optionally replacing either component. No slot, animation
or toast sound is started for a cancelled candidate.

## Toast families and edits

The families are `Advancement`, `Recipe`, `Tutorial`, `System`, `Mod` and `Other`
(including now-playing music). Match non-exhaustively. For advancement toasts, the title is
the task/goal/challenge label and the description is the advancement's name, not
its advancement-screen description. Recipe notifications keep their rotating
station/result icons; tutorial hints keep their progress bar. Edits change only
the displayed text, not these identities or lifetimes. Setting the optional
description to `None` removes that line.

## Admission rules

The hook runs for new additions and for each mod request. It does not run for
in-place `addOrUpdate` changes to an existing built-in recipe or system toast. A cancelled tutorial hint is not retried every frame;
a later, different hint is a new candidate. Title-screen backup/access failures,
narrator notices, pack download/failure and chunk I/O notices all use the same
admission point. A chunk I/O report produced while an integrated world is closing can
still be offered on the title screen of that same window.
A failed [delayed task](../world/delayed-tasks.md) on an integrated server
also offers a system toast there, only in the owning client's window. Editing or cancelling
it does not change the server's error log. Candidates produced during the initial resource
loading screen wait until the game can invoke the hook with a `ClientHost`. The pause screen's
persistent song label is HUD content, not a queued toast, and is not filtered by this hook.

A [panic diagnostic](../../concepts/hooks-and-the-host.md#failures-and-panics) uses ordinary
toast slots and animation timing and names the mod and callback. It bypasses `toast_add`, so
mods cannot cancel it or rewrite its text. A `render_gui_pre` veto and a mod's
`ClientHost::set_hud_mode` cannot hide it. F1 hides an audio stream failure notice, but not other callback error toasts. A dedicated server reports it in the log.
