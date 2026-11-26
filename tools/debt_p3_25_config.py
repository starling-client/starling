#!/usr/bin/env python3
"""Replay one mechanical slice of the P3-25 native config migration.

Run prepare-tests first. Run retire-hooks and retire-files separately when
migrating their callers; neither slice changes callback or fixture behavior.
"""

import argparse
import subprocess
from hashlib import sha256
from pathlib import Path
from textwrap import dedent


ROOT = Path(__file__).resolve().parents[1]
GAME = "crates/sunrise-client/src/game"
MODDING = f"{GAME}/modding"
VOCAB = "crates/sunrise-mod-vocab/src"
GEN = "crates/sunrise-mod-api-gen"


def replace(edits, path, old, new=""):
    source = edits[path] if path in edits else (ROOT / path).read_text(encoding="utf-8")
    if source.count(old) != 1:
        raise ValueError(f"{path}: expected exactly one match for {old.splitlines()[0]!r}")
    edits[path] = source.replace(old, new, 1)


def remove_between(edits, path, first, following):
    source = edits[path] if path in edits else (ROOT / path).read_text(encoding="utf-8")
    if source.count(first) != 1 or source.count(following) != 1:
        raise ValueError(f"{path}: expected unique boundaries {first!r} and {following!r}")
    start = source.index(first)
    end = source.index(following, start + len(first))
    replace(edits, path, source[start:end])


def remove_file(edits, path, expected):
    if sha256((ROOT / path).read_bytes()).hexdigest() != expected:
        raise ValueError(f"{path}: refusing to delete a file that differs from the reviewed base")
    edits[path] = None


def prepare_tests(edits):
    inline = "\n#[cfg(test)]\nmod tests {\n"
    for module in ("native_configs", "native_config_files"):
        parent = f"{MODDING}/{module}.rs"
        child = f"{MODDING}/{module}/tests.rs"
        source = (ROOT / parent).read_text(encoding="utf-8")
        if source.count(inline) != 1 or not source.endswith("\n}\n"):
            raise ValueError(f"{parent}: expected a single inline tests module at EOF")
        if (ROOT / child).exists():
            raise ValueError(f"{child}: refusing to overwrite the test module")
        prefix, tests = source.split(inline)
        body = tests.removesuffix("}\n")
        if any(line and not line.startswith("    ") for line in body.splitlines()):
            raise ValueError(f"{parent}: expected an indented tests body through EOF")
        # The module remains a child of the same Rust module; the path is
        # relative to its declaring source file, in modding/.
        edits[parent] = prefix + f'\n#[cfg(test)]\n#[path = "{module}/tests.rs"]\nmod tests;\n'
        edits[child] = subprocess.run(
            ["rustfmt", "--edition", "2024", "--emit", "stdout"],
            input=dedent(body),
            cwd=ROOT,
            stdout=subprocess.PIPE,
            text=True,
            check=True,
        ).stdout


def retire_hooks(edits):
    remove_file(
        edits,
        f"{VOCAB}/hooks/mod_configs.rs",
        "985500a9ce321d3a8820b02b3303d41d11457bbb8536f42ffafeef061e13ac6b",
    )
    remove_file(
        edits,
        f"{GAME}/native_hooks/mod_configs.rs",
        "9b939b318d88e8f84fde36d676258d723035f38739278e0a1f89eb8c8fa23477",
    )
    remove_file(
        edits,
        f"{GEN}/dispatch-tests/src/fixtures/mod_configs.rs",
        "40d6471175b56128957547a9b8343172418a36a587561b493886c570f81fb1da",
    )
    replace(edits, f"{GAME}/native_hooks/mod.rs", "    native_configs::ConfigFile,\n")
    replace(edits, f"{GAME}/native_hooks/mod.rs", "\nmod mod_configs;\n", "\n")
    replace(edits, f"{GAME}.rs", "\npub use native_configs::ConfigFile;\n", "\n")
    replace(
        edits,
        f"{GEN}/dispatch-tests/src/fixtures/shared.rs",
        "            Command, CommandRegistration, Config, DeclarationError, Dispensed, InteractionResult,\n"
        "            MapDecoration, MapInfo, Zoom,\n",
        "            Command, CommandRegistration, DeclarationError, Dispensed, InteractionResult,\n"
        "            MapDecoration, MapInfo, Zoom,\n",
    )
    remove_between(
        edits,
        f"{VOCAB}/game.rs",
        "\n/// A config a mod registers:",
        "\n/// A data map type a mod registers",
    )
    remove_between(
        edits,
        f"{VOCAB}/game.rs",
        "    // no wire: the in-process config loader builds this projection for callbacks.\n",
        "}\n\n/// Saved-world resolution records and listener-boundary attribution.\n",
    )
    replace(
        edits,
        f"{VOCAB}/exports/game.rs",
        "ColumnBlocks, Config, ConfigFile, ConfigKind",
        "ColumnBlocks, ConfigKind",
    )
    replace(
        edits,
        f"{VOCAB}/exports/game.rs",
        "        pub use crate::game::built::{ConfigFile, TerrainColumn};\n",
        "        pub use crate::game::built::TerrainColumn;\n",
    )


def retire_files(edits):
    remove_file(
        edits,
        f"{VOCAB}/content/config_files.rs",
        "895e979bb765f229befe9bba0e5c9ecfd1e870ba5bdacc334c69ad4cf8742ce5",
    )
    remove_file(
        edits,
        f"{VOCAB}/exports/config_files.rs",
        "d3269255181bf8c05cfad997b819bf7a9b8c09f56088b7768a30cf1916d9c92d",
    )
    remove_file(
        edits,
        f"{VOCAB}/trait_roles/ConfigFiles.call",
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    )
    replace(edits, f"{VOCAB}/content/mod.rs", "\npub(crate) mod config_files;\n", "\n")
    replace(edits, f"{GEN}/src/surface/mod.rs", '    ("ConfigFiles", "", ""),\n')

    wire = f"{VOCAB}/wire/wire_types.rs"
    remove_between(
        edits,
        wire,
        "impl Wire for crate::ConfigPath {\n",
        "impl Wire for crate::SpawnerKey {\n",
    )
    replace(
        edits,
        wire,
        '        round(crate::ConfigPath::new("example/options.toml").unwrap());\n'
        "        round(crate::ConfigPath::root());\n",
    )
    replace(
        edits,
        wire,
        '        out.put_str("../escape");\n'
        "        assert!(Reader::decode::<crate::ConfigPath>(out.bytes()).is_err());\n"
        "        out.clear();\n",
    )
    replace(
        edits,
        f"{VOCAB}/wire/wire_layout_tests.rs",
        "    fields(\n"
        '        ConfigPath::new("gear/options.toml").unwrap(),\n'
        '        ("gear/options.toml".to_owned(),),\n'
        "    );\n",
    )
    replace(
        edits,
        "tools/fixtures/wire_enum_baseline_identities.json",
        '  "crate::content::config_files::ConfigEntryKind",\n',
    )
    replace(
        edits,
        "tools/fixtures/wire_enum_layouts.json",
        "    {\n"
        '      "name": "ConfigEntryKind",\n'
        '      "path": "crate::content::config_files::ConfigEntryKind",\n'
        '      "hash": "f41f6f46c3f24fc5",\n'
        '      "canonical_path": "crate::content::config_files::ConfigEntryKind"\n'
        "    },\n",
    )

    # Remove the tests of the old proxy. Keep the two daily tests for hand
    # migration: they exercise the config reload and world lifecycle too.
    tests = f"{MODDING}/native_config_files/tests.rs"
    remove_between(
        edits,
        tests,
        "#[test]\nfn a_mod_writes_reads_lists_and_stats_its_own_files()",
        "/// The service through a real game and the owner-bound config listener:",
    )
    remove_between(
        edits,
        tests,
        "#[cfg(unix)]\n#[test]\nfn a_symbolic_link_never_leads_out_of_config()",
        '#[test]\n#[ignore = "daily"]\nfn world_bulk_files_survive_save_reload_backup_and_world_deletion()',
    )


def main():
    slices = {
        "prepare-tests": prepare_tests,
        "retire-hooks": retire_hooks,
        "retire-files": retire_files,
    }
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("slice", choices=slices)
    args = parser.parse_args()

    # Validate the entire selected slice before changing any files.
    edits = {}
    try:
        slices[args.slice](edits)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{args.slice}: {error}\n")
    for path, content in edits.items():
        target = ROOT / path
        if content is None:
            target.unlink()
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content, encoding="utf-8")


if __name__ == "__main__":
    main()
