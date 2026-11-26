//! Command refresh guidance carried from upstream; the command service owns the method.

pub trait Notes {
    #[notes = r#"`host.commands().refresh_commands()`, on the server's `Commands` service, asks every mod's `register_commands` again at the start of the next server tick, rebuilds the commands players may run, and sends every player on the server their tree again after `player_commands_sent`, as vanilla's `PlayerList.sendPlayerPermissionLevel` (`Commands.sendCommands`) does after `/reload`. Nothing happens where no world is open to players.

The new server dispatcher binds fresh executors to its accepted usages. A parse from the old dispatcher cannot execute after the rebuild. Players receive the rebuilt tree without its executors."#]
    fn refresh_commands();
}
