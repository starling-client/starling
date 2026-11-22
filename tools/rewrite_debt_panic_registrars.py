#!/usr/bin/env python3
"""Rewrite the native registrar crossings for the hook-panic debt item.

Run `ingress` for the concrete owner parameters, then `bridge <hook> ...` once
those hooks opt into the generated staged-surface adapter. Both are repeatable.
"""

import argparse
from pathlib import Path
import re


GAME = Path(__file__).resolve().parents[1] / "crates/sunrise-client/src/game"
NATIVE = GAME / "native_hooks"
# hook suffix -> (argument, registrar trait, concrete owner)
REGISTRARS = {
    "block_tint_sources": ("sources", "BlockTintSources", "BlockTintRegistry"),
    "color_resolvers": ("resolvers", "ColorResolvers", "ColorResolverRegistry"),
    "cauldron_fluid_content": ("contents", "CauldronFluidContents", "CauldronFluidContentRegistry"),
    "enchantment_damage_effects": ("effects", "EnchantmentDamageEffects", "EnchantmentDamageRegistry"),
    "villager_professions": ("professions", "VillagerProfessions", "ProfessionDeclarations"),
    "entity_renderers": ("renderers", "EntityRenderers", "EntityRendererRegistry"),
    "particle_providers": ("providers", "ParticleProviders", "ParticleProviderRegistry"),
    "special_model_renderer": ("renderers", "SpecialModelRenderers", "SpecialModelRendererRegistry"),
    "game_tests": ("functions", "GameTests", "GameTestFunctions"),
    "load_conditions": ("conditions", "LoadConditions", "LoadConditionDeclarations"),
    "entity_layers": ("layers", "EntityLayers", "LayerRegistration"),
    "client_item_extensions": ("extensions", "mod_api::ClientItemExtensionsRegistrar", "Registration"),
    "cauldron_interaction": ("interactions", "CauldronInteractions", "Registrar"),
}
GAME_OWNERS = {
    "entity_layers": ("modding/native_entity_layers.rs", "native_entity_layers", "pub(super) "),
    "client_item_extensions": ("modding/client_item_extensions.rs", "client_item_extensions", ""),
    "cauldron_interaction": ("blocks/cauldron_interactions.rs", "cauldron_interactions", ""),
}


def replace(source, old, new, path, *formatted, count=1):
    before = source.count(old)
    after = sum(source.count(pattern) for pattern in (new, *formatted))
    if (before, after) == (count, 0):
        return source.replace(old, new)
    if (before, after) == (0, count):
        return source
    raise SystemExit(f"{path}: expected exactly {count} crossing(s): {old!r} or {new!r}")


def rewrite_staged_calls(crossings, bridge):
    for hook, argument, trait, owner, prefix in crossings:
        path = NATIVE / f"{hook}.rs"
        source = replace(path.read_text(), f"{argument}: &mut dyn {trait}", f"{argument}: &mut {owner}", path)
        if bridge:
            call = f"fire_{hook}({prefix}"
            source = replace(source, f"{call}{argument});", f"{call}|listen| {argument}.register_listener(listen));", path,
                f"{call}|listen| {{\n            {argument}.register_listener(listen)\n        }});",
                f"{call}\n            |listen| {argument}.register_listener(listen),\n        );")
        path.write_text(source)


def reload_hud_and_tickets(bridge):
    rewrite_staged_calls((
        ("add_client_reload_listeners", "listeners", "ClientReloadListeners", "mod_api::ClientListenerRegistry", ""),
        ("add_server_reload_listeners", "listeners", "ServerReloadListeners", "mod_api::ServerListenerRegistry", ""),
        ("register_gui_layers", "layers", "GuiLayerRegistrar", "sunrise_client::gui_layers::GuiLayers", "context, "),
        ("validate_tickets", "validation", "mod_api::TicketValidation", "mod_api::world::mod_chunk_tickets::ControllerTickets", "dimension, controller, "),
    ), bridge)
    if not bridge:
        return

    path = GAME / "interface/client_reload/mod.rs"
    path.write_text(replace(
        path.read_text(),
        "mod_api::dispatch_add_client_reload_listeners(&StartupClientHost, &mut listeners);",
        "mod_api::dispatch_add_client_reload_listeners(&StartupClientHost, |listen| {\n            listeners.register_listener(listen);\n        });",
        path,
    ))
    path = GAME.parents[2] / "sunrise-mod-api/src/load_state.rs"
    path.write_text(replace(
        path.read_text(),
        "dispatch_add_server_reload_listeners(&host, &mut ServerListenerRegistry::new());",
        "let mut listeners = ServerListenerRegistry::new();\n                dispatch_add_server_reload_listeners(&host, |listen| listeners.register_listener(listen));",
        path,
        "let mut listeners = ServerListenerRegistry::new();\n                dispatch_add_server_reload_listeners(&host, |listen| {\n                    listeners.register_listener(listen)\n                });",
    ))


def pack_section_and_poi(bridge):
    rewrite_staged_calls((
        ("add_pack_finders", "packs", "mod_api::PackFinders", "crate::game::pack_finders::Registry", ""),
        ("add_section_geometry", "renderers", "mod_api::SectionGeometryRenderers", "mod_api::SectionGeometryRegistry", "context, "),
        ("extend_poi_types", "extensions", "PoiTypeExtensions", "mod_api::PoiTypeExtensionRegistry<'_>", "context, "),
    ), bridge)
    path = GAME / "modding/pack_finders/mod.rs"
    source = replace(path.read_text(), "\nstruct Registry {", "\npub(in crate::game) struct Registry {", path)
    if bridge:
        source = replace(
            source,
            "mod_api::dispatch_add_pack_finders(host, &mut registry);",
            "mod_api::dispatch_add_pack_finders(host, |listen| registry.register_listener(listen));",
            path,
        )
    path.write_text(source)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("ingress")
    reload_hud_tickets = commands.add_parser("reload-hud-tickets")
    reload_hud_tickets.add_argument("--bridge", action="store_true")
    pack_section_poi = commands.add_parser("pack-section-poi")
    pack_section_poi.add_argument("--bridge", action="store_true")
    bridge = commands.add_parser("bridge")
    bridge.add_argument("hooks", nargs="+", choices=REGISTRARS)
    args = parser.parse_args()
    if args.command == "reload-hud-tickets":
        reload_hud_and_tickets(args.bridge)
        return

    if args.command == "pack-section-poi":
        pack_section_and_poi(args.bridge)
        return

    pending = {}
    for suffix in REGISTRARS if args.command == "ingress" else args.hooks:
        argument, trait, owner = REGISTRARS[suffix]
        hook = f"register_{suffix}"
        path = NATIVE / f"{hook}.rs"
        source = path.read_text()
        formatted = ()
        if args.command == "ingress":
            if suffix == "game_tests":
                source = replace(source, "use mod_api::GameTests;\n", "use mod_api::GameTestFunctions;\n", path)
                old = f"fn native_{hook}(&self, {argument}: &mut dyn {trait})"
                new = f"fn native_{hook}(&self, {argument}: &mut {owner})"
            else:
                if suffix == "load_conditions":
                    source = replace(source, "use super::*;\nuse mod_api::LoadConditions;\nimpl", "use super::*;\nimpl", path)
                prefix = f"fn native_{hook}(\n        context: &mod_api::RegistrationContext,\n        "
                old = f"{prefix}{argument}: &mut dyn {trait},"
                if suffix in GAME_OWNERS:
                    owner_file, module, visibility = GAME_OWNERS[suffix]
                    owner_path = GAME / owner_file
                    pending[owner_path] = replace(
                        owner_path.read_text(),
                        f"\n{visibility}struct {owner}<'a>",
                        f"\npub(in crate::game) struct {owner}<'a>",
                        owner_path,
                    )
                    gap = "\n" if suffix == "entity_layers" else ""
                    source = replace(
                        source,
                        f"use super::*;\n{gap}impl Game",
                        f"use super::*;\nuse crate::game::{module}::{owner};\n{gap}impl Game",
                        path,
                    )
                    new = f"{prefix}{argument}: &mut {owner}<'_>,"
                else:
                    new = f"{prefix}{argument}: &mut mod_api::{owner},"
        else:
            prefix = f"self.fire_{hook}(" if suffix == "game_tests" else f"Self::fire_{hook}(context, "
            old = f"{prefix}{argument});"
            new = f"{prefix}|listen| {argument}.register_listener(listen));"
            formatted = (
                f"{prefix}|listen| {{\n            {argument}.register_listener(listen)\n        }});",
            )
        pending[path] = replace(source, old, new, path, *formatted)

    if args.command == "bridge" and "client_item_extensions" in args.hooks:
        path = GAME / "modding/client_item_extensions.rs"
        source = path.read_text()
        new = r"let mut registry = Registration::new(\1);"
        source, count = re.subn(
            r"let mut registry = Registration \{\s*state: Default::default\(\),\s*"
            r"sprite_exists: ([^\n]+),\s*refused: None,\s*\};",
            new, source,
        )
        if count not in (0, 5) or source.count("let mut registry = Registration::new(") != 5:
            raise SystemExit(f"{path}: expected five item-extension fixture constructors")
        pending[path] = source

    if args.command == "bridge" and "cauldron_interaction" in args.hooks:
        path = GAME / "blocks/cauldron_interactions/tests.rs"
        source = path.read_text()
        source, count = re.subn(
            r"Registrar \{\s*table: Table::default\(\),\s*items: ([^\n]+),\s*"
            r"registry: ([^\n]+),\s*all: Vec::new\(\),\s*error: None,\s*\}",
            r"Registrar::new(\1, \2)", source,
        )
        if count not in (0, 6) or source.count("Registrar::new(") < 6:
            raise SystemExit(f"{path}: expected six cauldron fixture constructors")
        pending[path] = source

    if args.command == "bridge" and "load_conditions" in args.hooks:
        path = GAME / "modding/load_conditions/tests.rs"
        call = "mod_api::dispatch_register_load_conditions(&RegistrationContext::new(), "
        pending[path] = replace(
            path.read_text(),
            call + "&mut registrar);",
            call + "|listen| {\n                registrar.register_listener(listen)\n            });",
            path,
            count=2,
        )

    for path, source in pending.items():
        if source != path.read_text():
            path.write_text(source)


if __name__ == "__main__":
    main()
