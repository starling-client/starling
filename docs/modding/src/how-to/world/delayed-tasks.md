# Delayed tasks

Use `host.schedule_task` to run a closure after a delay in server ticks. Like vanilla's
`MinecraftServer.schedule(TickTask)`, it takes a callable. Move the data the task needs into the
closure; it cannot borrow the current hook's host or event. When the task runs, it receives a
fresh `&mut dyn ServerHost` on the same thread.

```rust,ignore
use mod_api::{ScheduleError, ServerHost, TaskId, Text};

fn schedule_reminder(host: &mut dyn ServerHost) -> Result<TaskId, ScheduleError> {
    let reminder = Text::translate("mymod.reminder", vec![]);
    // Keep the returned id if you might need to call host.cancel_task(id).
    host.schedule_task(20, Box::new(move |host| {
        let _ = host.messages().broadcast(reminder);
    }))
}
```

**Timing.** Delays count ticks the server runs: a paused integrated server does not advance them.
Due tasks drain to exhaustion after both the `server_tick` start phase and the `server_tick` end
phase (matching vanilla's `MinecraftServer.schedule(TickTask)` and
`BlockableEventLoop.runAllTasks`), so `delay_ticks == 0` runs in the current tick: a `0`-delay task
scheduled from `server_tick` start, mid-tick hooks, `server_tick` end, or another running task
completes within the active drain without advancing the tick. Tasks due on the same tick run in the
order they were scheduled.

**Ownership and cancellation.** Only the scheduled callable runs. When scheduled inside a mod call,
it runs under that mod's identity (including when it schedules more work or when its captures drop
on cancellation or world close). When due, the game drops the task without calling it if its owner
or that owner's delayed callbacks are disabled. Scheduling outside an active mod call also
succeeds. `TaskId` is an opaque capability (mods cannot construct one): any holder of a
still-pending `id` can cancel it with
`host.cancel_task(id)`, which returns `true` when the task was still waiting. A cancelled or
discarded task releases the values it captured; ids are never reused, even across world resets.

**Refusals.** `schedule_task` returns `ScheduleError::TooMany` (and logs a warning naming the mod)
when the scheduling mod already has `MAX_PENDING_TASKS` tasks waiting (unattributed callers outside
a mod callback share no global cap), and `ScheduleError::Unavailable` outside the world phase or
without an open or opening server world.

**Writes and failures.** Each callback runs in its own transaction. Journaled writes commit on
normal return and roll back on panic or a phase refusal. If a mod's callback panics, the game names
that mod in the log and sends a system toast to the integrated server owner's window. The owning
client can edit or cancel the toast through [`toast_add`](../client/toast-notifications.md); the
server log is unaffected. The mod's other delayed callbacks stop for the rest of the session, even
if you reopen the world. Its other hooks are unaffected, the tick continues, and other mods' due
tasks still run. A phase refusal is logged each time and does not disable further tasks.

If a capture's destructor panics on cancellation or world close, the game disables the
**scheduling** mod's delayed callbacks for the same session, even when another mod cancelled the
task. The error names the scheduling mod in the log and shows a system toast in the owning
window without calling `toast_add`. `cancel_task` still returns `true`, and world close still
drops the remaining tasks and finishes cleanup.

**Lifetime.** The queue belongs to the current world and is never saved. Closing the world or a
failed open drops all its pending tasks. If a deadline needs to outlive a restart, save the deadline
in your mod's own storage and schedule a new closure when the world opens.

## Spare time in a tick

`host.server_has_time()` says whether time remains before the next server tick is due (NeoForge's `ServerTickEvent.hasTime` and `LevelTickEvent.hasTime`, vanilla's `MinecraftServer.haveTime`).
A listener of `server_tick` or of a level tick (`level_tick_pre`, `level_tick_post`) that has optional work, such
as a cache rebuild, checks it and leaves the rest for a later tick when it is false. It is always true on a
client and outside a scheduled server tick, so the same code runs on both sides.

```rust,ignore
fn level_tick_pre(&self, host: HostSide, _: &mut LevelTickPreEvent) {
    if let HostSide::Server(host) = host
        && host.server_has_time()
    {
        self.rebuild_next_cache_entry();
    }
}
```
