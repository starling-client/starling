//! The `register_commands` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0032

pub trait Hooks {
    /// The commands native mods add to the server's command tree: push each usage with
    /// its executor attached by [`Command::executes`]. Asked whenever the server builds
    /// its command tree, as the reference loader's
    /// `Commands` does: when a world opens (a dedicated server, a LAN host, single player's
    /// server thread), before its functions load, so a function line may name your command;
    /// on each `/reload` and data pack change, so a command you stop pushing is refused by the
    /// next functions loaded; and after a mod calls `host.commands().refresh_commands()`.
    /// Every player's chat box completes them as vanilla's, within their level. A
    /// root vanilla owns is left out. A lower-level child opens traversal through a
    /// prefix without changing that prefix's own execution permission. When accepted usages
    /// execute at the same node (including optional prefixes), each later accepted registration
    /// overwrites that node's executor (matching Brigadier's `CommandNode.addChild`, which sets
    /// `child.command = node.getCommand()`) together with Sunrise's paired execution permission
    /// level for that node. Only that node's executor runs, attributed to its mod; if the
    /// owner is disabled, execution is refused rather than handed to another mod.
    /// Clients get the command tree, not executors. An argument's kind is one
    /// of `word`, `text`, `integer`, `number`, `boolean`, `player`, `item`,
    /// `block_pos` and `vec3`, or a command argument type a mod registered
    /// (`Registrar::register_command_argument`), named by its namespaced id.
    #[notes = r#"## Command argument kinds beyond the basics

- Besides `word`, `text`, `integer`, `number`, `boolean`, `player`, `item` and `block_pos`, an argument's kind may be `vec3`, `vec3_uncentered`, `column_pos`, `rotation`, `dimension` or `nbt`. Each is sent to clients as vanilla's parser, so `~`, `^` and the dimension list complete as in a vanilla command. A `vec3` and a `vec3_uncentered` cannot share a position in one command's usages, as they look the same to a client.

- `vec3` and `vec3_uncentered` deliver `CommandArgumentValue::Vec3`; `column_pos` delivers `ColumnPos`, `rotation` delivers `Rotation` (yaw, pitch), `dimension` delivers `Dimension(LevelRef)` and `nbt` delivers `Nbt(Compound)`. `~` and `^` resolve against the source's position and rotation. A `dimension` the world has not loaded is refused with `argument.dimension.invalid` before the executor runs."#]
    #[hook(
        target = "sunrise_client::game::native_hooks::base::Game<'_>::native_register_commands",
        dispatch = "flat",
        side = "server",
        contract = "collect",
        out = "commands",
        answered_by = "mod",
        neoforge = "RegisterCommandsEvent",
        version = 4
    )]
    fn register_commands(commands: &mut Vec<mod_api::Answered<mod_api::CommandRegistration>>);
}
