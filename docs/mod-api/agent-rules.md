# For agents

Rules for a coding agent making a Sunrise mod. They are short on purpose; each links to its page.

1. **Choose the shape.** A content mod has no `Cargo.toml`. A code mod has `Cargo.toml` and `src/` at the top and
   implements `Hooks` with `#[mod_api::sunrise_mod]`. Prefer the typed API to source edits.
2. **Use a hook or a Part method** when one exists (search the rustdoc of `sunrise-mod-api`). A per-block,
   per-item or per-entity moment is a Part method ([Parts](../modding/src/concepts/parts.md)).
3. **Change game code only when needed.** Use the [native route](../modding/src/concepts/native-route.md)
   when the API cannot express the change. A [`[[native.mixin]]`](../modding/src/concepts/native-route/2-mixins.md)
   patches a function against the prebuilt SDK.
4. **Names:** NeoForge 26.3 names for hooks that have a counterpart; no `on_` prefix; the event is `&mut`;
   handles come from the event; writes take `host`.
5. **Layout:** `sunrise-mod.toml`, `assets/`, `data/`, `Cargo.toml` and `src/` at the top. The id and version come
   only from `sunrise-mod.toml`; ids are `[a-z0-9_]`. No `native/` folder, no `sunrise-native.json`.
6. **Manifest fields:** `id`, `name`, `version`, `description`, `authors`, `api = "0.2"`, `side`,
   `capabilities = ["registries"]`, `[dependencies]`, `[incompatible]`, `[discouraged]`, `load_before`, top-level `internals`, `[native]`
   (`game`, `sunrise`, `mixins`, optional `mixin`, `loop`, `bridge`, `module`; `state` is refused; see [the native route](../modding/src/concepts/native-route.md)) and `[[jar]]`. Unknown fields are refused
   ([manifest](../modding/src/reference/manifest.md)).
7. **Freshness errors:** check changed internals against the pinned source before refreshing their declarations
   with [`internals add` or `internals sync`](../modding/src/reference/registry-index.md#native-dependency-tools).
8. **Data and assets** are read through a reload hook, once per reload, never on the tick.
9. **Missing hook?** File a [hook request](https://github.com/sunrise-org/sunrise/issues/new) naming the
   game action, side, required values and whether the hook would observe, change or cancel the action.
