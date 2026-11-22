//! What `cargo sunrise-mod --help`, `help all`, `help <command>` and `<command> --help` print: one
//! table, every command once.

struct Command {
    name: &'static str,
    /// One of the commands a mod author uses every day: listed by plain `--help`.
    everyday: bool,
    usage: &'static str,
    summary: &'static str,
}

const COMMANDS: &[Command] = &[
    Command {
        name: "new",
        everyday: true,
        usage: "new <path> [--name \"Name\"] [--template content|native [--sunrise <build>]]",
        summary: "Makes a mod project in <path>; a native one is pinned to the current Nightly's SDK unless --sunrise names another build.",
    },
    Command {
        name: "build",
        everyday: true,
        usage: "build [--update-lock] [--binaries <dir> --sunrise-release <tag> --releases <file> [--server]] | build --describe-binaries <dir> --sunrise-release <tag> --releases <file> --sdk-data <dir>",
        summary: "Packs the mod into target/sunrise-mod/<id>.smod; a native mod is compiled first (--update-lock resolves its Cargo.lock again). --binaries also writes the current SDK host's archive (--server selects the dedicated server); --describe-binaries assembles a descriptor from signed host archives without compiling.",
    },
    Command {
        name: "run",
        everyday: true,
        usage: "run [--game <sunrise-client>] [--resources <folder>] [--jars <dir>] [-- <game args>]",
        summary: "Builds the mod, starts the game, and shows a native mod's own log lines live; --game names the game for a content mod, --resources the prepared resources for a native mod (which runs on its pinned SDK's game), and --jars selects a folder of original Java jars named by SHA-512.",
    },
    Command {
        name: "check",
        everyday: true,
        usage: "check [--uses] | check <file> [--kind <kind>] [--key <public key> --signature <file.minisig>] [--json]",
        summary: "Checks the mod in this folder, or a file as the registry would; --uses lists what the mod uses of its pinned build, and a native mod's Rust code is checked on this computer.",
    },
    Command {
        name: "update",
        everyday: true,
        usage: "update [--sunrise <build>]",
        summary: "Repins a native mod to another game build (the current Nightly unless --sunrise names one) and reports what that breaks; a content mod pins no build.",
    },
    Command {
        name: "hooks",
        everyday: true,
        usage: "hooks <text> [--all]",
        summary: "Searches the pinned build's hooks, or with --all every item of its API, offline.",
    },
    Command {
        name: "publish",
        everyday: false,
        usage: "publish [--out <dir>] [--registry <checkout>] [--summary <text>] [--owner <login>]… [--categories a,b] [--tags a,b] [--changelog <text>] [--key <public key> --signature <file.minisig>] [--package <file.smod>] [--sunrise <tag>] [--asset-base <author release URL> --releases <file>] [--binaries <dir> --sdk-data <dir>]",
        summary: "Stages a package, its registry entry, and pull request text; it uploads nothing. --sunrise pins signed source to a stable release from --releases and uses the author’s release at --asset-base. --binaries checks signed release assets and fit; to append to an existing version, give --registry and its original signed --package.",
    },
    Command {
        name: "release",
        everyday: false,
        usage: crate::release::USAGE,
        summary: "`release plan` selects a tagged native release or a rebuild and stages verified SDK metadata. `release prepare-rebuild` verifies the original reviewed source and writes a scratch manifest or project without running Cargo. `release scripts` writes the shipped workflow scripts.",
    },
    Command {
        name: "keygen",
        everyday: false,
        usage: "keygen [--out <dir>] [--no-password]",
        summary: "Makes a Minisign key pair in the mod’s per-user key folder (or --out) and prints the public key. The private key is encrypted unless --no-password is given (Unix only). Never overwrites either key.",
    },
    Command {
        name: "sign",
        everyday: false,
        usage: "sign <file> [--key <private key path>] [--no-password] | sign <file> --key-env",
        summary: "Writes the detached .minisig beside <file> without replacing it. Uses the current mod’s per-user signing key unless --key is given; --key-env reads SUNRISE_MOD_SIGNING_KEY and optional SUNRISE_MOD_SIGNING_PASSPHRASE without prompting.",
    },
    Command {
        name: "internals",
        everyday: false,
        usage: "internals sync | internals add <path>… [--kind <kind>]",
        summary: "Writes the game internals a native mod names into sunrise-mod.toml, found by checking its code or taken from the pinned build's catalog.",
    },
    Command {
        name: "sdk",
        everyday: false,
        usage: "sdk diff <build> <build>",
        summary: "Lists what changed in the API surface between two builds.",
    },
    Command {
        name: "fits",
        everyday: false,
        usage: "fits [<file.smod>] [--build <name>]…",
        summary: "Says whether the mod fits each build; exits 0 when all do, 1 when one does not and 2 when that cannot be told.",
    },
    Command {
        name: "registry",
        everyday: false,
        usage: "registry build|review|check|list|release|page | registry releases --out <releases.json>",
        summary: "The registry repository's offline CI commands, and `releases` to fetch the stable Sunrise release list.",
    },
    Command {
        name: "use-jar",
        everyday: false,
        usage: "use-jar <path>",
        summary: "Lists an exact original Java jar as a passive asset and data source and caches it for runs.",
    },
    Command {
        name: "native-build",
        everyday: false,
        usage: "native-build < plan.json",
        summary: "Compiles native mods against an unpacked SDK from a JSON plan on standard input (reference/sdk/08-native-build.md).",
    },
    Command {
        name: "native-check",
        everyday: false,
        usage: "native-check < plan.json",
        summary: "Says how native mods stand against one build before anything compiles.",
    },
    Command {
        name: "native-discover",
        everyday: false,
        usage: "native-discover < plan.json",
        summary: "Finds the game internals a native mod names, or the pinned game's build data.",
    },
    Command {
        name: "native-lock",
        everyday: false,
        usage: "native-lock < plan.json",
        summary: "Writes a native mod's registry lock from its pinned game's build data.",
    },
    Command {
        name: "native-layout",
        everyday: false,
        usage: "native-layout < plan.json",
        summary: "Checks a local prebuilt game and lays it out as an SDK's slices.",
    },
];

/// The short list `cargo sunrise-mod --help` prints.
pub fn short() -> String {
    let mut out = String::from("usage: cargo sunrise-mod <command> [options]\n\n");
    for command in COMMANDS.iter().filter(|c| c.everyday) {
        let first = command.summary.split(';').next().unwrap_or(command.summary);
        out += &format!("  {:<8}{}\n", command.name, first.trim_end_matches('.'));
    }
    out += "\n`cargo sunrise-mod <command> --help` says more about one command.\nmore: cargo sunrise-mod help all\n";
    out
}

/// Every command's usage, one per line: `help all`.
pub fn all() -> String {
    COMMANDS
        .iter()
        .map(|c| format!("cargo sunrise-mod {}\n", c.usage))
        .collect()
}

/// One command's usage and what it does, when `name` is a command.
pub fn of(name: &str) -> Option<String> {
    let command = COMMANDS.iter().find(|c| c.name == name)?;
    Some(format!(
        "usage: cargo sunrise-mod {}\n\n{}\n",
        command.usage, command.summary
    ))
}

/// The text for a `help` command's arguments, or why it names no command.
pub fn run(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        None => Ok(short()),
        Some("all") => Ok(all()),
        Some(name) => of(name).ok_or_else(|| unknown(name)),
    }
}

/// The error for a word that is not a command.
pub fn unknown(name: &str) -> String {
    format!("{name:?} is not a cargo sunrise-mod command; `cargo sunrise-mod --help` lists them")
}

/// Whether `args` (a command's, after its name) ask for its help: `--help` or `-h` right after the
/// command. Later it may be an option's value (`--summary --help`) or meant for the game (`run -- --help`).
pub fn asked(args: &[String]) -> bool {
    args.first().is_some_and(|a| a == "--help" || a == "-h")
}
