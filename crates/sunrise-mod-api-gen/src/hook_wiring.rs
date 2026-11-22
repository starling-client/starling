//! The source-only CI check for catalog targets. Build generation never calls the check.
mod calls;
mod sources;

use crate::{Catalog, Hook, catalog::DispatchStyle, dispatch, free_function_source, source};
pub(super) use calls::{Callee, has_call_matching};
use calls::{Routes, Target, path_is};
use sources::{Build, Sources};
use std::path::Path;
use syn::{Block, Expr, Item};

/// Check that each declared hook target reaches its adapter or dispatcher. Never writes sources.
/// Each entry in `target_cfgs` is one supported build's `rustc --print cfg` output.
pub fn check_hook_wiring(root: &Path, target_cfgs: &[String]) -> Result<(), String> {
    check_with_builds(
        root,
        &Catalog::load(root)?.hooks,
        Build::from_targets(target_cfgs)?,
    )
}

#[cfg(test)]
pub(super) fn check(root: &Path, hooks: &[Hook]) -> Result<(), String> {
    let targets = [
        "unix\ntarget_family=\"unix\"\ntarget_os=\"linux\"".into(),
        "windows\ntarget_family=\"windows\"\ntarget_os=\"windows\"".into(),
    ];
    check_with_builds(root, hooks, Build::from_targets(&targets).unwrap())
}

fn check_with_builds(root: &Path, hooks: &[Hook], initial: Build) -> Result<(), String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("{}: {error}", root.display()))?;
    let mut sources = Sources::new(&root);
    let errors: Vec<_> = hooks
        .iter()
        .filter_map(|hook| {
            check_builds(&mut sources, hook, &initial)
                .err()
                .map(|error| format!("{} ({}): {error}", hook.name, hook.target))
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn check_builds(sources: &mut Sources<'_>, hook: &Hook, initial: &Build) -> Result<(), String> {
    let mut found = false;
    let mut pending = vec![initial.clone()];
    while let Some(build) = pending.pop() {
        let result = check_target(sources, hook, &build);
        if let Some(builds) = build.split() {
            pending.extend(builds);
        } else {
            found |= result?;
        }
    }
    if found {
        Ok(())
    } else {
        Err("target definition not found".into())
    }
}

fn functions<'a>(
    items: impl Iterator<Item = &'a Item>,
    name: &'a str,
    build: &'a Build,
) -> impl Iterator<Item = &'a Block> {
    items.filter_map(move |item| match item {
        Item::Fn(function) if function.sig.ident == name && build.enabled(&function.attrs) => {
            Some(function.block.as_ref())
        }
        _ => None,
    })
}

const NATIVE_HOOKS: &str = "sunrise_client::game::native_hooks::";

fn check_target(sources: &mut Sources<'_>, hook: &Hook, build: &Build) -> Result<bool, String> {
    let (package, module, owner, target) = if let Some(path) =
        hook.target.strip_prefix(NATIVE_HOOKS)
    {
        let (module_and_owner, target) = path.rsplit_once("::").ok_or("expected a method")?;
        let (module, owner) = module_and_owner
            .rsplit_once("::")
            .ok_or("expected a method owner")?;
        (
            "sunrise_client".to_owned(),
            format!("game::native_hooks::{module}"),
            Some(owner),
            target,
        )
    } else {
        let (package, path) = free_function_source(&hook.target).ok_or("unknown target crate")?;
        let (module, target) = path.rsplit_once("::").ok_or("expected a free function")?;
        (package.replace('-', "_"), module.to_owned(), None, target)
    };
    let Some(items) = sources.items(build, &package, &module)? else {
        return Ok(false);
    };
    let bodies = if let Some(owner) = owner {
        items
            .filter_map(|item| match item {
                Item::Impl(item)
                    if item.trait_.is_none()
                        && source::owner(&item.self_ty) == owner
                        && build.enabled(&item.attrs) =>
                {
                    Some(&item.items)
                }
                _ => None,
            })
            .flatten()
            .filter_map(|item| match item {
                syn::ImplItem::Fn(method)
                    if method.sig.ident == target && build.enabled(&method.attrs) =>
                {
                    Some(&method.block)
                }
                _ => None,
            })
            .cloned()
            .collect::<Vec<_>>()
    } else {
        functions(items, target, build).cloned().collect()
    };
    if bodies.is_empty() {
        return Err("target definition not found".into());
    }
    let name = &hook.name;
    let mut index = sources.index(build);
    let mut routes = Routes::new(&mut index, build);
    if dispatch::is_render(hook) {
        // The raw target is an intentionally empty replacement slot. Start at the bound
        // renderer callback, not a helper elsewhere in the module.
        check_slot(
            &mut routes,
            build,
            SlotTable {
                package: "sunrise_mod_render",
                module: &module,
                installer: "install",
                install: Target::Definition {
                    package: "sunrise_client_render",
                    module: "render_slots",
                    name: "install",
                },
                slots: "engine::Slots",
            },
            name,
            Target::Path(&format!("generated::fire_{name}")),
        )?;
        return Ok(true);
    }
    let scope = routes.module(&package, &module)?;
    let bodies: Vec<_> = bodies
        .iter()
        .map(|body| routes.calls(scope).in_block(body))
        .collect();
    if hook.target == crate::catalog::EFFECT_TICK_TARGET {
        require_calls(&bodies, Some(name), &[])?;
        return Ok(true);
    }
    if owner.is_some() && hook.dispatch != DispatchStyle::Direct {
        let fire = format!("fire_{name}");
        require_calls(&bodies, Some(&fire), &[&format!("Self::{fire}")])?;
        return Ok(true);
    }
    let direct = format!("mod_api::dispatch_{name}");
    if package == "sunrise_mod_vocab" {
        require_calls(&bodies, None, &[&format!("crate::dispatch_{name}")])?;
        return Ok(true);
    }
    let (bridge, table, installer, slots) = match package.as_str() {
        "sunrise_client_lib" => (
            "native_client_hooks",
            "NativeClientHooks",
            "install_client_hooks",
            "crate::native_client_hooks::NativeClientHooks",
        ),
        "sunrise_world" => (
            "native_load_hooks",
            "NativeLoadHooks",
            "start",
            "sunrise_world::native_load_hooks::NativeLoadHooks",
        ),
        _ => {
            require_calls(&bodies, None, &[&direct])?;
            return Ok(true);
        }
    };
    let wrapper_path = format!("crate::{bridge}::{name}");
    let outgoing = bodies
        .iter()
        .any(|body| calls(body, None, &[&wrapper_path]));
    if package == "sunrise_client_lib" {
        require_calls(&bodies, None, &[&direct, &wrapper_path])?;
    } else {
        require_calls(&bodies, None, &[&wrapper_path])?;
    }
    // These client-library slots precede their catalog targets. The gameplay entry is
    // part of the route even when the slot declaration is accidentally deleted.
    let incoming = if package == "sunrise_client_lib" {
        match name.as_str() {
            "can_grindstone_repair" => Some(("stations", "can_grindstone_repair")),
            "is_combine_repairable" => Some(("crafting::repair_item", "combine_repairable")),
            "can_fit_inside_container_items" => {
                Some(("menu::container_items", "fits_inside_container_items"))
            }
            _ => None,
        }
    } else {
        None
    };
    if let Some((entry_module, entry_name)) = incoming {
        let scope = routes.module(&package, entry_module)?;
        let entries = routes.functions(scope, entry_name);
        let wrapper = Target::Definition {
            package: &package,
            module: bridge,
            name,
        };
        if entries.is_empty()
            || entries
                .iter()
                .any(|body| !routes.block_reaches(scope, body, wrapper))
        {
            return Err(format!(
                "{entry_module}::{entry_name} does not call {wrapper}"
            ));
        }
    }
    if outgoing || incoming.is_some() {
        let scope = routes.module(&package, bridge)?;
        if !routes.has_slot(scope, table, name) {
            return Err(format!("{bridge}::{table} is missing its {name} slot"));
        }
        let wrappers = routes.functions(scope, name);
        let invokes_slot = !wrappers.is_empty()
            && wrappers.iter().all(|body| {
                routes
                    .calls(scope)
                    .in_block(body)
                    .into_iter()
                    .any(|invocation| {
                        let Expr::Call(call) = invocation.expr else {
                            return false;
                        };
                        let Expr::Field(field) = calls::ungroup(&call.func) else {
                            return false;
                        };
                        matches!(&field.member, syn::Member::Named(member) if member == name)
                    })
            });
        if !invokes_slot {
            return Err(format!(
                "{bridge}::{name} does not invoke its callback slot"
            ));
        }
        let destination = if incoming.is_some() {
            Target::Definition {
                package: &package,
                module: &module,
                name: target,
            }
        } else {
            Target::Path(&direct)
        };
        check_slot(
            &mut routes,
            build,
            SlotTable {
                package: "sunrise_client",
                module: "entry",
                installer,
                install: Target::Definition {
                    package: &package,
                    module: bridge,
                    name: "install",
                },
                slots,
            },
            name,
            destination,
        )?;
    }
    Ok(true)
}

fn require_calls(
    bodies: &[Vec<calls::Invocation<'_>>],
    method: Option<&str>,
    paths: &[&str],
) -> Result<(), String> {
    if bodies.iter().any(|body| !calls(body, method, paths)) {
        let expected = method
            .into_iter()
            .chain(paths.iter().copied())
            .collect::<Vec<_>>()
            .join(" or ");
        return Err(format!("target does not call {expected}"));
    }
    Ok(())
}

fn calls(body: &[calls::Invocation<'_>], method: Option<&str>, paths: &[&str]) -> bool {
    body.iter()
        .filter_map(|invocation| Callee::from_expr(invocation.expr))
        .any(|call| match call {
            Callee::Method(called) => method.is_some_and(|method| called == method),
            Callee::Function(called) => paths.iter().any(|path| path_is(called, path)),
        })
}

/// Where the executable binds a table of engine callbacks.
struct SlotTable<'a> {
    package: &'static str,
    module: &'a str,
    installer: &'a str,
    install: Target<'a>,
    slots: &'a str,
}

/// Check the field in every table actually passed to the installer in this build.
fn check_slot(
    routes: &mut Routes<'_, '_, '_>,
    build: &Build,
    SlotTable {
        package,
        module,
        installer,
        install,
        slots,
    }: SlotTable<'_>,
    name: &str,
    target: Target<'_>,
) -> Result<(), String> {
    let scope = routes.module(package, module)?;
    let installers = routes.functions(scope, installer);
    let missing = || format!("{module}::{installer} does not install {slots}::{name}");
    if installers.is_empty() {
        return Err(missing());
    }
    for body in &installers {
        let installed = routes.check_bindings(scope, body, install, |routes, scope, call, imports| {
            let Some(Expr::Struct(table)) = call.args.first().map(calls::ungroup) else {
                return Err(format!(
                    "{module}::{installer}: cannot inspect the {slots} binding passed to {install}"
                ));
            };
            let field = table.fields.iter().find(|field| {
                matches!(&field.member, syn::Member::Named(member) if member == name)
                    && build.enabled(&field.attrs)
            }).ok_or_else(missing)?;
            if !routes.callback_reaches(scope, imports, &field.expr, target) {
                return Err(format!(
                    "{module}::{installer}: installed {slots}::{name} does not call {target}"
                ));
            }
            Ok(())
        })?;
        if !installed {
            return Err(missing());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Side;
    use std::fs;

    fn fixture(root: &Path, path: &str, text: &str) {
        let path = root.join("crates").join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    const PICKUP: &str = "sunrise-client/src/game/native_hooks/pickup.rs";

    fn pickup(root: &Path, declarations: &str) -> Hook {
        fixture(root, "sunrise-client/Cargo.toml", "");
        fixture(root, "sunrise-client/src/lib.rs", "mod game;");
        fixture(root, "sunrise-client/src/game.rs", "mod native_hooks;");
        fixture(
            root,
            "sunrise-client/src/game/native_hooks/mod.rs",
            declarations,
        );
        let mut hook = Hook::sample(
            "sunrise_client::game::native_hooks::pickup::Game<'_>::native_item_picked_up",
            Side::Server,
        );
        hook.name = "item_picked_up".into();
        hook
    }

    #[test]
    fn wiring_check_requires_a_call_in_the_declared_target() {
        let root = tempfile::tempdir().unwrap();
        let mut hook = pickup(root.path(), "mod pickup;");
        for (dispatch, body, expected) in [
            (DispatchStyle::Flat, "self.fire_item_picked_up(item);", true),
            (
                DispatchStyle::Flat,
                "let _ = (item); // self.fire_item_picked_up(item);\n",
                false,
            ),
            (
                DispatchStyle::Flat,
                "fn unused() { Self::fire_item_picked_up(item); }",
                false,
            ),
            (
                DispatchStyle::Flat,
                "mod_api::dispatch_item_picked_up(item);",
                false,
            ),
            (
                DispatchStyle::Direct,
                "other::dispatch_item_picked_up(item);",
                false,
            ),
            (
                DispatchStyle::Direct,
                "mod_api::dispatch_item_picked_up(item);",
                true,
            ),
        ] {
            let text = format!(
                "impl Game<'_> {{ fn native_item_picked_up(&self, item: u32) {{ {body} }} fn unrelated(&self) {{ self.fire_item_picked_up(1); }} }}"
            );
            fixture(root.path(), PICKUP, &text);
            hook.dispatch = dispatch;
            let result = check(root.path(), std::slice::from_ref(&hook));
            assert_eq!(result.is_ok(), expected, "{body}: {result:?}");
            assert_eq!(
                fs::read_to_string(root.path().join("crates").join(PICKUP)).unwrap(),
                text
            );
        }
    }

    #[test]
    fn test_only_pickup_calls_do_not_wire_the_production_target() {
        let root = tempfile::tempdir().unwrap();
        let hook = pickup(root.path(), "mod pickup;");
        for (body, wired) in [
            ("#[cfg(test)] { self.fire_item_picked_up(item); }", false),
            ("#[cfg(test)] self.fire_item_picked_up(item);", false),
            (
                "#[cfg(test)] let _test = self.fire_item_picked_up(item);",
                false,
            ),
            (
                "#[cfg(all(test, unix))] { self.fire_item_picked_up(item); }",
                false,
            ),
            (
                "#[cfg_attr(not(test), cfg(any()))] { self.fire_item_picked_up(item); }",
                false,
            ),
            (
                "fn at_pickup(f: impl FnOnce()) { f(); } at_pickup(|| self.fire_item_picked_up(item));",
                true,
            ),
        ] {
            fixture(
                root.path(),
                PICKUP,
                &format!(
                    "impl Game<'_> {{ fn native_item_picked_up(&self, item: u32) {{ {body} }} }}"
                ),
            );
            let result = check(root.path(), std::slice::from_ref(&hook));
            assert_eq!(result.is_ok(), wired, "{body}: {result:?}");
        }
    }

    #[test]
    fn pickup_can_use_a_module_level_closure_invoker() {
        let root = tempfile::tempdir().unwrap();
        let hook = pickup(root.path(), "mod pickup;");
        for (helper, wired) in [("drop(f);", false), ("f();", true)] {
            fixture(
                root.path(),
                PICKUP,
                &format!(
                    "
                fn at_pickup(f: impl FnOnce()) {{ {helper} }}
                impl Game<'_> {{
                    fn native_item_picked_up(&self) {{ at_pickup(|| self.fire_item_picked_up()); }}
                }}
            "
                ),
            );
            let result = check(root.path(), std::slice::from_ref(&hook));
            assert_eq!(result.is_ok(), wired, "{helper}: {result:?}");
        }
    }

    #[test]
    fn pickup_uses_its_declared_path_or_inline_body() {
        let root = tempfile::tempdir().unwrap();
        let native = "sunrise-client/src/game/native_hooks";
        let hook = pickup(
            root.path(),
            "#[cfg(test)] mod pickup; #[path = \"pickup/live.rs\"] mod pickup;",
        );
        fixture(
            root.path(),
            PICKUP,
            "impl Game<'_> { fn native_item_picked_up(&self) {} }",
        );
        let wired =
            "impl Game<'_> { fn native_item_picked_up(&self) { self.fire_item_picked_up(); } }";
        fixture(root.path(), &format!("{native}/pickup/live.rs"), wired);
        check(root.path(), std::slice::from_ref(&hook)).unwrap();
        fixture(
            root.path(),
            &format!("{native}/mod.rs"),
            &format!("mod pickup {{ {wired} }}"),
        );
        check(root.path(), &[hook]).unwrap();
    }

    #[test]
    fn pickup_checks_the_default_when_the_path_override_only_applies_to_tests() {
        let root = tempfile::tempdir().unwrap();
        let hook = pickup(
            root.path(),
            "#[cfg_attr(test, path = \"pickup/test.rs\")] mod pickup;",
        );
        let disconnected = "impl Game<'_> { fn native_item_picked_up(&self) {} }";
        let wired =
            "impl Game<'_> { fn native_item_picked_up(&self) { self.fire_item_picked_up(); } }";
        let alternate = "sunrise-client/src/game/native_hooks/pickup/test.rs";
        fixture(root.path(), alternate, wired);
        fixture(root.path(), PICKUP, disconnected);
        let error = check(root.path(), std::slice::from_ref(&hook)).unwrap_err();
        assert!(
            error.contains("target does not call fire_item_picked_up"),
            "{error}"
        );
        fixture(root.path(), alternate, disconnected);
        fixture(root.path(), PICKUP, wired);
        check(root.path(), &[hook]).unwrap();
    }

    #[test]
    fn pickup_checks_inline_and_file_module_variants() {
        let root = tempfile::tempdir().unwrap();
        let wired =
            "impl Game<'_> { fn native_item_picked_up(&self) { self.fire_item_picked_up(); } }";
        fixture(root.path(), PICKUP, wired);
        let hook = pickup(
            root.path(),
            "#[cfg(unix)] mod pickup; #[cfg(not(unix))] mod pickup {
            impl Game<'_> { fn native_item_picked_up(&self) {} }
        }",
        );
        let error = check(root.path(), std::slice::from_ref(&hook)).unwrap_err();
        assert!(
            error.contains("target does not call fire_item_picked_up"),
            "{error}"
        );
        pickup(
            root.path(),
            &format!("#[cfg(unix)] mod pickup; #[cfg(not(unix))] mod pickup {{ {wired} }}"),
        );
        check(root.path(), &[hook]).unwrap();
    }

    const RENDER: &str = "sunrise-mod-render/src/render.rs";

    fn render_needs(root: &Path) -> Hook {
        fixture(root, "sunrise-mod-render/Cargo.toml", "");
        fixture(root, "sunrise-mod-render/src/lib.rs", "mod render;");
        fixture(root, "sunrise-client-render/Cargo.toml", "");
        fixture(
            root,
            "sunrise-client-render/src/lib.rs",
            "pub mod render_slots {
            pub struct Slots { pub render_needs: fn(&mut RenderNeeds) }
            pub fn install(_: Slots) {}
        }",
        );
        let mut hook = Hook::sample("mod_render::render::render_needs", Side::Client);
        hook.name = "render_needs".into();
        hook
    }

    #[test]
    fn render_check_requires_the_installed_slot_not_an_unrelated_caller() {
        let root = tempfile::tempdir().unwrap();
        let hook = render_needs(root.path());
        for (callback, wired) in [
            ("|needs| { generated::fire_render_needs(needs); }", true),
            ("|_needs| {}", false),
            (
                "|needs| { if cfg!(test) { generated::fire_render_needs(needs); } }",
                false,
            ),
            (
                "|needs| { if cfg!(test) && { generated::fire_render_needs(needs); true } {} }",
                false,
            ),
            (
                "|needs| { if !cfg!(test) { generated::fire_render_needs(needs); } }",
                true,
            ),
            (
                "|needs| { if cfg!(test) {} else { generated::fire_render_needs(needs); } }",
                true,
            ),
            (
                "|needs| { let _unused = || generated::fire_render_needs(needs); }",
                false,
            ),
            (
                "|needs| { let forward = || generated::fire_render_needs(needs); forward(); }",
                true,
            ),
            ("|needs| run(|| generated::fire_render_needs(needs))", true),
            (
                "|needs| discard(|| generated::fire_render_needs(needs))",
                false,
            ),
        ] {
            fixture(
                root.path(),
                RENDER,
                &format!(
                    r"
                fn render_needs(_: &mut RenderNeeds) {{}}
                fn needs() {{ generated::fire_render_needs(&mut RenderNeeds::default()); }}
                fn run(f: impl FnOnce()) {{ f(); }}
                fn discard(_f: impl FnOnce()) {{}}
                fn install() {{
                    use sunrise_client_render::render_slots as engine;
                    engine::install(engine::Slots {{ render_needs: {callback} }});
                }}
            "
                ),
            );
            let result = check(root.path(), std::slice::from_ref(&hook));
            if wired {
                result.unwrap();
            } else {
                assert_eq!(
                    result.unwrap_err(),
                    "render_needs (mod_render::render::render_needs): render::install: installed engine::Slots::render_needs does not call generated::fire_render_needs"
                );
            }
        }
    }

    #[test]
    fn render_checks_every_invoked_production_binding() {
        let root = tempfile::tempdir().unwrap();
        let hook = render_needs(root.path());
        let wired =
            "engine::install(engine::Slots { render_needs: generated::fire_render_needs });";
        let disconnected = "engine::install(engine::Slots { render_needs: |_| {} });";
        for (body, error) in [
            (
                format!("#[cfg(unix)] {{ {wired} }} #[cfg(not(unix))] {{ {disconnected} }}"),
                Some("does not call generated::fire_render_needs"),
            ),
            (
                format!("{wired} {disconnected}"),
                Some("does not call generated::fire_render_needs"),
            ),
            (
                format!(
                    "{wired} let other = engine::Slots {{ render_needs: |_| {{}} }}; engine::install(other);"
                ),
                Some("cannot inspect the engine::Slots binding"),
            ),
            (
                format!("let _unused = || {{ {wired} }};"),
                Some("does not install engine::Slots::render_needs"),
            ),
            (
                format!("#[cfg(unix)] {{ {wired} }} #[cfg(not(unix))] {{ {wired} }}"),
                None,
            ),
            (
                format!(
                    r#"
                    #[cfg(unix)] {{ {wired} }}
                    #[cfg(not(target_family = "unix"))] {{ {wired} }}
                    #[cfg(all(target_os = "linux", target_os = "windows"))] {{ {disconnected} }}
                "#
                ),
                None,
            ),
            (
                format!(
                    "use engine::install as bind; bind(engine::Slots {{ render_needs: |_| {{}} }}); {wired}"
                ),
                Some("does not call generated::fire_render_needs"),
            ),
            (
                format!("fn bind_first() {{ {disconnected} }} bind_first(); {wired}"),
                Some("does not call generated::fire_render_needs"),
            ),
            (
                format!("bind_first(); {wired}"),
                Some("does not call generated::fire_render_needs"),
            ),
            (
                format!(
                    "use engine::install as bind; let table = engine::Slots {{ render_needs: |_| {{}} }}; bind(table); {wired}"
                ),
                Some("cannot inspect the engine::Slots binding"),
            ),
            ("bind_wired();".to_owned(), None),
            (format!("let bind = || {{ {wired} }}; bind();"), None),
            (format!("run(|| {{ {wired} }});"), None),
        ] {
            fixture(
                root.path(),
                RENDER,
                &format!(
                    "
                    use sunrise_client_render::render_slots as engine;
                    fn render_needs() {{}}
                    fn run(f: impl FnOnce()) {{ f(); }}
                    fn bind_first() {{ {disconnected} }}
                    fn bind_wired() {{ {wired} }}
                    fn install() {{ {body} }}
                "
                ),
            );
            let result = check(root.path(), std::slice::from_ref(&hook));
            if let Some(error) = error {
                let actual = result.unwrap_err();
                assert!(actual.contains(error), "{body}: {actual}");
            } else {
                result.unwrap();
            }
        }
    }

    #[test]
    fn client_and_world_wrappers_require_their_installed_dispatchers() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fixture(
            root,
            "sunrise-client-lib/Cargo.toml",
            "[lib]\npath = \"../sunrise-client/src/client_lib.rs\"\n",
        );
        fixture(
            root,
            "sunrise-client/src/client_lib.rs",
            "mod hunger; pub mod native_client_hooks;",
        );
        fixture(
            root,
            "sunrise-client/src/hunger.rs",
            "fn native_living_heal() { crate::native_client_hooks::living_heal(); }",
        );
        fixture(
            root,
            "sunrise-client/src/native_client_hooks.rs",
            "pub struct NativeClientHooks { pub living_heal: fn() }
             pub fn install(_: NativeClientHooks) {}
             fn living_heal(hooks: &NativeClientHooks) { (hooks.living_heal)(); }",
        );
        fixture(root, "sunrise-client/Cargo.toml", "");
        fixture(
            root,
            "sunrise-client/src/lib.rs",
            "mod entry; mod game; pub use sunrise_client_lib::*;",
        );
        fixture(
            root,
            "sunrise-client/src/game.rs",
            "mod heal; pub(crate) use heal::*; pub(crate) use heal::heal_callback as renamed;",
        );
        fixture(
            root,
            "sunrise-client/src/game/heal.rs",
            "pub(crate) fn heal_callback() { mod_api::dispatch_living_heal(); }",
        );
        fixture(root, "sunrise-world/Cargo.toml", "");
        fixture(
            root,
            "sunrise-world/src/lib.rs",
            "mod loot; pub mod native_load_hooks;",
        );
        fixture(
            root,
            "sunrise-world/src/loot.rs",
            "fn native_loot_table_load() { crate::native_load_hooks::loot_table_load(); }",
        );
        fixture(
            root,
            "sunrise-world/src/native_load_hooks.rs",
            "pub struct NativeLoadHooks { pub loot_table_load: fn() }
             pub fn install(_: NativeLoadHooks) {}
             fn loot_table_load(hooks: &NativeLoadHooks) { (hooks.loot_table_load)(); }",
        );
        let mut heal = Hook::sample(
            "sunrise_client_lib::hunger::native_living_heal",
            Side::Server,
        );
        heal.name = "living_heal".into();
        let mut load = Hook::sample("sunrise_world::loot::native_loot_table_load", Side::Server);
        load.name = "loot_table_load".into();
        let hooks = [heal, load];
        for (client, world, bad_hook) in [
            (
                "|| mod_api::dispatch_living_heal()",
                "|| mod_api::dispatch_loot_table_load()",
                None,
            ),
            (
                "game::heal_callback",
                "|| mod_api::dispatch_loot_table_load()",
                None,
            ),
            (
                "|| crate::game::heal_callback()",
                "|| mod_api::dispatch_loot_table_load()",
                None,
            ),
            (
                "game::renamed",
                "|| mod_api::dispatch_loot_table_load()",
                None,
            ),
            (
                "|| {}",
                "|| mod_api::dispatch_loot_table_load()",
                Some("living_heal"),
            ),
            (
                "|| mod_api::dispatch_living_heal()",
                "|| {}",
                Some("loot_table_load"),
            ),
        ] {
            fixture(
                root,
                "sunrise-client/src/entry.rs",
                &format!(
                    r"
                use crate::{{game}};
                fn install_client_hooks() {{
                    crate::native_client_hooks::install(crate::native_client_hooks::NativeClientHooks {{ living_heal: {client} }});
                }}
                fn start() {{
                    install_client_hooks();
                    sunrise_world::native_load_hooks::install(sunrise_world::native_load_hooks::NativeLoadHooks {{ loot_table_load: {world} }});
                }}
            "
                ),
            );
            let result = check(root, &hooks);
            if let Some(name) = bad_hook {
                let error = result.unwrap_err();
                assert!(error.starts_with(&format!("{name} (")), "{error}");
                assert!(
                    error.ends_with(&format!("::{name} does not call mod_api::dispatch_{name}")),
                    "{error}"
                );
                assert!(!error.contains('\n'), "{error}");
            } else {
                result.unwrap();
            }
        }
    }

    #[test]
    fn direct_client_target_also_requires_its_incoming_slot() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path();
        fixture(
            root,
            "sunrise-client-lib/Cargo.toml",
            "[lib]\npath = \"../sunrise-client/src/client_lib.rs\"\n",
        );
        fixture(
            root,
            "sunrise-client/src/client_lib.rs",
            "pub mod stations; pub mod native_client_hooks;",
        );
        const STATIONS: &str = "sunrise-client/src/stations.rs";
        const GAMEPLAY: &str = "pub fn can_grindstone_repair() { crate::native_client_hooks::can_grindstone_repair(); }";
        const DISPATCH: &str =
            "pub fn native_can_grindstone_repair() { mod_api::dispatch_can_grindstone_repair(); }";
        fixture(root, STATIONS, &format!("{GAMEPLAY}\n{DISPATCH}"));
        const BRIDGE: &str = "sunrise-client/src/native_client_hooks.rs";
        const BRIDGE_BODY: &str = "
            pub fn install(_: NativeClientHooks) {}
            pub(crate) fn can_grindstone_repair(hooks: &NativeClientHooks) { (hooks.can_grindstone_repair)(); }
        ";
        fixture(
            root,
            BRIDGE,
            &format!(
                "pub struct NativeClientHooks {{ can_grindstone_repair: fn() }}\n{BRIDGE_BODY}"
            ),
        );
        fixture(root, "sunrise-client/Cargo.toml", "");
        fixture(
            root,
            "sunrise-client/src/lib.rs",
            "mod entry; pub use sunrise_client_lib::*;",
        );
        let mut hook = Hook::sample(
            "sunrise_client_lib::stations::native_can_grindstone_repair",
            Side::Server,
        );
        hook.name = "can_grindstone_repair".into();
        for (adapter, wired) in [
            ("mod_api::dispatch_can_grindstone_repair();", false),
            ("", false),
            (
                "sunrise_client::stations::native_can_grindstone_repair();",
                true,
            ),
        ] {
            fixture(root, "sunrise-client/src/entry.rs", &format!("
                fn adapter() {{ {adapter} }}
                fn install_client_hooks() {{
                    crate::native_client_hooks::install(crate::native_client_hooks::NativeClientHooks {{
                        can_grindstone_repair: adapter,
                    }});
                }}
            "));
            let result = check(root, std::slice::from_ref(&hook));
            if wired {
                result.unwrap();
            } else {
                let error = result.unwrap_err();
                assert!(
                    error.contains(
                        "does not call sunrise_client_lib::stations::native_can_grindstone_repair"
                    ),
                    "{error}"
                );
            }
        }
        fixture(
            root,
            STATIONS,
            &format!("pub fn can_grindstone_repair() {{}}\n{DISPATCH}"),
        );
        let error = check(root, std::slice::from_ref(&hook)).unwrap_err();
        assert!(error.contains("stations::can_grindstone_repair does not call sunrise_client_lib::native_client_hooks::can_grindstone_repair"), "{error}");
        fixture(root, STATIONS, &format!("{GAMEPLAY}\n{DISPATCH}"));
        fixture(
            root,
            BRIDGE,
            &format!("pub struct NativeClientHooks {{}}\n{BRIDGE_BODY}"),
        );
        let error = check(root, &[hook]).unwrap_err();
        assert!(
            error.contains("NativeClientHooks is missing its can_grindstone_repair slot"),
            "{error}"
        );
    }

    #[test]
    fn wiring_check_requires_each_shared_effect_callback() {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path(), "sunrise-mod-vocab/Cargo.toml", "");
        fixture(
            root.path(),
            "sunrise-mod-vocab/src/lib.rs",
            "mod parts { mod effects; }",
        );
        const EFFECTS: &str = "sunrise-mod-vocab/src/parts/effects.rs";
        fixture(
            root.path(),
            EFFECTS,
            "fn dispatch_mob_effect_tick() { part.should_apply_effect_tick_this_tick(); }",
        );
        let mut cadence = Hook::sample(crate::catalog::EFFECT_TICK_TARGET, Side::Server);
        cadence.name = "should_apply_effect_tick_this_tick".into();
        let mut apply = cadence.clone();
        apply.name = "apply_effect_tick".into();
        let hooks = [cadence, apply];
        assert_eq!(
            check(root.path(), &hooks).unwrap_err(),
            "apply_effect_tick (mod_api::parts::effects::dispatch_mob_effect_tick): target does not call apply_effect_tick"
        );
        for (advance, expected) in [("", false), ("due.next();", true)] {
            fixture(root.path(), EFFECTS, &format!("
                fn dispatch_mob_effect_tick() {{
                    let mut due = crate::entered_parts().filter(|part| part.should_apply_effect_tick_this_tick());
                    {advance}
                    part.apply_effect_tick();
                }}
            "));
            let result = check(root.path(), &hooks);
            if expected {
                result.unwrap();
            } else {
                assert_eq!(
                    result.unwrap_err(),
                    "should_apply_effect_tick_this_tick (mod_api::parts::effects::dispatch_mob_effect_tick): target does not call should_apply_effect_tick_this_tick"
                );
            }
        }
        fixture(
            root.path(),
            "sunrise-mod-vocab/src/lib.rs",
            "mod parts { mod effects; } mod runtime;",
        );
        fixture(
            root.path(),
            EFFECTS,
            "fn dispatch_mob_effect_tick() {
                part.should_apply_effect_tick_this_tick();
                crate::runtime::invoke(|| part.apply_effect_tick());
            }",
        );
        for (fence, expected) in [
            (
                "std::panic::catch_unwind(std::panic::AssertUnwindSafe(call));",
                true,
            ),
            ("drop(std::panic::AssertUnwindSafe(call));", false),
            ("invoke(call);", false),
        ] {
            fixture(
                root.path(),
                "sunrise-mod-vocab/src/runtime.rs",
                &format!(
                    "
                pub(crate) fn invoke(call: impl FnOnce()) {{ contain(call); }}
                fn contain(call: impl FnOnce()) {{ {fence} }}
            "
                ),
            );
            let result = check(root.path(), &hooks);
            assert_eq!(result.is_ok(), expected, "{fence}: {result:?}");
        }
    }

    #[test]
    fn wiring_check_ignores_unreached_feature_gated_items_when_splitting_builds() {
        let root = tempfile::tempdir().unwrap();
        fixture(root.path(), "sunrise-world/Cargo.toml", "");
        fixture(
            root.path(),
            "sunrise-world/src/lib.rs",
            "pub mod loot; pub mod native_load_hooks;",
        );
        fixture(
            root.path(),
            "sunrise-world/src/loot.rs",
            "pub fn load() { crate::native_load_hooks::loot_table_load(); }",
        );
        fixture(
            root.path(),
            "sunrise-world/src/native_load_hooks.rs",
            "pub struct NativeLoadHooks { pub loot_table_load: fn() }\n\
             pub fn install(_: NativeLoadHooks) {}\n\
             pub fn loot_table_load() { (hooks.loot_table_load)(); }",
        );
        fixture(root.path(), "sunrise-client/Cargo.toml", "");
        fixture(
            root.path(),
            "sunrise-client/src/lib.rs",
            "mod entry; mod helper;",
        );
        fixture(
            root.path(),
            "sunrise-client/src/entry.rs",
            "pub fn start() {\n\
                sunrise_world::native_load_hooks::install(\n\
                    sunrise_world::native_load_hooks::NativeLoadHooks {\n\
                        loot_table_load: || { mod_api::dispatch_loot_table_load(); },\n\
                    },\n\
                );\n\
                crate::helper::unrelated();\n\
             }",
        );
        fixture(
            root.path(),
            "sunrise-client/src/helper.rs",
            "#[cfg(feature = \"unused_feature_a\")]\n\
             pub fn never_called() {}\n\
             #[cfg(feature = \"unused_feature_b\")]\n\
             pub struct UnusedType;\n\
             #[cfg(feature = \"unused_feature_c\")]\n\
             use std::fmt::Debug;\n\
             #[cfg(feature = \"unused_feature_d\")]\n\
             mod unused_mod;\n\
             pub fn unrelated() {}",
        );
        let mut hook = Hook::sample("sunrise_world::loot::load", Side::Server);
        hook.name = "loot_table_load".into();
        let root_canon = root.path().canonicalize().unwrap();
        let mut sources = Sources::new(&root_canon);
        let targets = [
            "unix\ntarget_family=\"unix\"\ntarget_os=\"linux\"".into(),
            "windows\ntarget_family=\"windows\"\ntarget_os=\"windows\"".into(),
        ];
        let build = Build::from_targets(&targets).unwrap();
        assert!(check_target(&mut sources, &hook, &build).unwrap());
        assert!(
            build.split().is_none(),
            "unreached #[cfg(feature = ...)] items must not trigger a build split"
        );
    }
}
