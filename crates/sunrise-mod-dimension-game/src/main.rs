//! The game with a native mod that adds levels, for `tools/mod_dimensions_e2e.py`. It is what the
//! launcher's generated `main` builds, with one mod: `dimprobe` registers the level
//! `dimprobe:pocket` on the game's noise, the chunk generator `dimprobe:slab`, and the level
//! `dimprobe:slabs` it fills. The server command `dimprobe enter <level>` sends its player there
//! with `run_command`, inside the command, and `dimprobe later <level>` from the next server tick,
//! where no player's state is swapped in. The run needs a `sunrise-native-mods/dimprobe` package
//! beside the executable, which the harness writes.
//!
//! A crate and not an example of sunrise-client: an example builds with the dev-dependencies, whose
//! `testing` feature keeps the mods per thread, and the server thread would see none.
use mod_api::__private::native::BuiltMod;
use mod_api::Side;
use mod_api::{
    BlockKey, ChunkAccess, ChunkGenerator, Command, CommandArgumentValue, CommandInvocation,
    CommandSource, DimensionSettings, Hooks, ModEntry, RegisterCommandsEvent, Registrar,
    RegistrationContext, ServerHost, ServerTickEvent, ServerTickPhase, Text,
};
use std::sync::{Arc, Mutex};

const BUILT: &[BuiltMod] = &[BuiltMod {
    id: "dimprobe",
    version: "1.0.0",
    side: Side::Both,
}];

/// A stone floor up to y=0 under a gold block at each chunk's corner, its y the world's seed
/// modulo 16 above the floor: terrain only this generator makes.
struct Slab {
    stone: BlockKey,
    gold: BlockKey,
}

impl ChunkGenerator for Slab {
    fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
        let stone = chunk.state(self.stone);
        let gold = chunk.state(self.gold);
        let [x, z] = [chunk.pos().x * 16, chunk.pos().z * 16];
        for (dx, dz) in (0..16).flat_map(|dx| (0..16).map(move |dz| (dx, dz))) {
            for y in chunk.min_y()..=0 {
                chunk.set_block([x + dx, y, z + dz], stone);
            }
        }
        let marker = 1 + chunk.seed().rem_euclid(16) as i32;
        chunk.set_block([x, marker, z], gold);
    }
}

#[derive(Default)]
struct DimProbe {
    /// The line the next server tick runs.
    later: Arc<Mutex<Option<String>>>,
}

/// The command that sends the issuing player into the named level, 40 blocks over y=0.
fn travel(
    host: &dyn ServerHost,
    source: &CommandSource,
    command: &CommandInvocation,
) -> Result<String, Box<Text>> {
    let (CommandSource::Player { player, .. }, Some(CommandArgumentValue::String(level))) =
        (source, command.argument("level"))
    else {
        return Err(Text::literal("Expected a player and a level").into());
    };
    let name = player
        .name(host)
        .ok_or_else(|| Text::literal("The player is no longer available"))?;
    Ok(format!("execute in dimprobe:{level} run tp {name} 8 40 8"))
}

#[mod_api::__subscriptions]
impl Hooks for DimProbe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let stone = r.block_key("minecraft:stone");
        let gold = r.block_key("minecraft:gold_block");
        r.register_chunk_generator("slab", Slab { stone, gold });
        r.register_dimension(
            "pocket",
            DimensionSettings {
                min_y: -128,
                height: 448,
                logical_height: 448,
                ..Default::default()
            },
        );
        r.register_dimension(
            "slabs",
            DimensionSettings {
                min_y: -32,
                height: 128,
                logical_height: 128,
                generator: "dimprobe:slab".into(),
                biome: "minecraft:desert".into(),
                ..Default::default()
            },
        );
    }

    fn register_commands(&self, _: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        let later = self.later.clone();
        event.commands.extend([
            Command::new("dimprobe enter <level:word>").executes(|host, source, command| {
                let line = travel(host, source, command)?;
                let ran = host.commands().run_command(&line);
                host.commands()
                    .feedback(Text::literal(format!("dimprobe ran={ran}")), false)
                    .expect("bounded probe text");
                Ok(1)
            }),
            Command::new("dimprobe later <level:word>").executes(move |host, source, command| {
                let line = travel(host, source, command)?;
                *later.lock().unwrap() = Some(line);
                host.commands()
                    .feedback(Text::literal("dimprobe queued"), false)
                    .expect("bounded probe text");
                Ok(1)
            }),
        ]);
    }

    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if matches!(event.phase, ServerTickPhase::ServerTickStart { .. })
            && let Some(line) = self.later.lock().unwrap().take()
        {
            let ran = host.commands().run_command(&line);
            log::info!("dimprobe later ran={ran}");
        }
    }
}

fn main() {
    let startup = sunrise_client::entry::prepare();
    sunrise_mod_host::main_with(
        vec![ModEntry::new("dimprobe", Box::<DimProbe>::default())],
        BUILT,
        startup,
    );
}
