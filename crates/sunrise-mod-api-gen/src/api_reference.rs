//! `docs/modding/src/reference/api-reference.md` from the hook catalog and host area traits.
//!
//! The complete rustdoc still lives in `cargo doc -p sunrise-mod-api --no-deps --open`; this
//! generated Markdown chapter indexes the catalog-backed `Hooks` and Part hooks and the
//! area-declared `Host` / `ServerHost` / `ClientHost` methods directly in the book and on
//! `playsunrise.net` without checking generated Markdown into the repository.
use crate::catalog::{Catalog, Contract, Hook, PartKind, Side};
use crate::host::{self, Tier};
use quote::ToTokens;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

/// Renders the API reference chapter from the loaded catalog and host areas.
pub(crate) fn render(catalog: &Catalog, areas: &[host::Area]) -> String {
    let mut global_hooks = Vec::new();
    let mut part_hooks: BTreeMap<&'static str, Vec<&Hook>> = BTreeMap::new();
    for hook in &catalog.hooks {
        if crate::dispatch::is_render(hook) {
            continue;
        }
        if let Some(scope) = &hook.scope {
            part_hooks
                .entry(part_trait(scope.kind))
                .or_default()
                .push(hook);
        } else {
            global_hooks.push(hook);
            if let Some(part) = hook.part {
                part_hooks.entry(part_trait(part)).or_default().push(hook);
            }
        }
    }
    global_hooks.sort_by(|a, b| a.name.cmp(&b.name));
    for hooks in part_hooks.values_mut() {
        hooks.sort_by(|a, b| a.name.cmp(&b.name));
    }

    let mut page = String::from(
        "# Mod API reference\n\n\
This page is a generated index of the catalog-backed trait hooks (`crates/sunrise-mod-vocab/src/hooks/`) and\n\
area-declared host methods (`crates/sunrise-mod-vocab/src/host/`). For complete trait definitions (including\n\
non-catalog methods), handles, and cross-linked rustdoc, build the crate docs locally with\n\
`cargo doc -p sunrise-mod-api --no-deps --open`.\n\n\
- [Hooks (`Hooks` trait)](#hooks-hooks-trait)\n\
- [Part hooks](#part-hooks)\n\
- [Host methods (`Host`, `ServerHost`, `ClientHost`)](#host-methods-host-serverhost-clienthost)\n",
    );

    page.push_str("\n## Hooks (`Hooks` trait)\n\n");
    page.push_str(
        "| Hook | Side | Contract | Host / context | NeoForge | Summary |\n| --- | --- | --- | --- | --- | --- |\n",
    );
    for hook in &global_hooks {
        write_hook_row(&mut page, hook);
    }

    page.push_str("\n## Part hooks\n");
    for (trait_name, hooks) in &part_hooks {
        let _ = writeln!(page, "\n### `{trait_name}`\n");
        page.push_str(
            "| Hook | Side | Contract | Host / context | NeoForge | Summary |\n| --- | --- | --- | --- | --- | --- |\n",
        );
        for hook in hooks {
            write_hook_row(&mut page, hook);
        }
    }

    page.push_str("\n## Host methods (`Host`, `ServerHost`, `ClientHost`)\n");
    for (tier, heading) in [
        (Tier::Host, "Shared (`Host`)"),
        (Tier::ServerHost, "Server (`ServerHost`)"),
        (Tier::ClientHost, "Client (`ClientHost`)"),
    ] {
        let mut rows = Vec::new();
        for area in areas {
            let Some(entries) = area.tiers.get(&tier) else {
                continue;
            };
            for entry in entries {
                if is_doc_hidden(&entry.attrs) {
                    continue;
                }
                let sig = format_signature(&entry.sig);
                let summary = first_doc_sentence(&entry.attrs);
                rows.push((entry.name.as_str(), sig, summary));
            }
        }
        rows.sort_by(|a, b| a.0.cmp(b.0));
        let _ = writeln!(page, "\n### {heading}\n");
        page.push_str("| Method | Signature | Summary |\n| --- | --- | --- |\n");
        for (name, sig, summary) in rows {
            let _ = writeln!(
                page,
                "| `{}` | `{}` | {} |",
                escape_cell(name),
                escape_cell(&sig),
                escape_cell(&summary),
            );
        }
    }

    page
}

/// Reads the checkout's declarations and renders the API reference page.
pub fn generate(root: &Path) -> Result<String, String> {
    let catalog = Catalog::load(root).map_err(|error| error.to_string())?;
    let areas = host::load(root)?;
    Ok(render(&catalog, &areas))
}

fn write_hook_row(out: &mut String, hook: &Hook) {
    let host_or_ctx = hook_host_or_context(hook);
    let neoforge = hook_neoforge(hook);
    let summary = first_sentence(&hook.doc);
    let _ = writeln!(
        out,
        "| `{}` | {} | {} | {} | {} | {} |",
        escape_cell(&hook.name),
        side_label(hook.side),
        contract_label(hook.contract),
        host_or_ctx,
        neoforge,
        escape_cell(&summary),
    );
}

fn hook_host_or_context(hook: &Hook) -> String {
    if let Some(param) = crate::dispatch::listener_host_or_context(hook) {
        return format!("`{}`", escape_cell(&param.ty));
    }
    "—".into()
}

fn hook_neoforge(hook: &Hook) -> String {
    if hook.counterparts.neoforge.is_empty() {
        return "—".into();
    }
    let joined = hook.counterparts.neoforge.join(", ");
    if joined == "none" || joined.starts_with("none ") || joined.starts_with("none(") {
        return "—".into();
    }
    format!("`{}`", escape_cell(&joined))
}

fn side_label(side: Side) -> &'static str {
    match side {
        Side::Server => "server",
        Side::Client => "client",
        Side::Both => "both",
    }
}

fn contract_label(contract: Contract) -> &'static str {
    match contract {
        Contract::Listen => "listen",
        Contract::Cancel => "cancel",
        Contract::Collect => "collect",
        Contract::Replace => "replace",
    }
}

fn part_trait(kind: PartKind) -> &'static str {
    match kind {
        PartKind::Block => "BlockPart",
        PartKind::BlockEntity => "BlockEntityPart",
        PartKind::Item => "ItemPart",
        PartKind::Menu => "MenuPart",
        PartKind::Effect => "MobEffectPart",
        PartKind::Fluid => "FluidPart",
        PartKind::Entity => "EntityPart",
    }
}

fn is_doc_hidden(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("doc")
            && attr
                .meta
                .require_list()
                .ok()
                .is_some_and(|list| list.tokens.to_string().contains("hidden"))
    })
}

fn format_signature(sig: &syn::Signature) -> String {
    let raw = sig.to_token_stream().to_string();
    // Collapse token spacing into readable Rust signatures.
    raw.replace(" & ", " &")
        .replace("& mut ", "&mut ")
        .replace(" , ", ", ")
        .replace(" : ", ": ")
        .replace(" ( ", "(")
        .replace(" )", ")")
        .replace("( ", "(")
        .replace(" :: ", "::")
        .replace(" < ", "<")
        .replace(" >", ">")
        .replace(" ; ", "; ")
        .replace("& self", "&self")
        .trim()
        .to_string()
}

fn first_doc_sentence(attrs: &[syn::Attribute]) -> String {
    let mut lines = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident("doc")) {
        if let syn::Meta::NameValue(nv) = &attr.meta
            && let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) = &nv.value
        {
            lines.push(s.value());
        }
    }
    first_sentence(&lines.join("\n"))
}

fn first_sentence(doc: &str) -> String {
    let mut paragraph = Vec::new();
    for line in doc.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !paragraph.is_empty() {
                break;
            }
            continue;
        }
        // Skip metadata lead-ins such as "Side: server." if followed by description.
        if paragraph.is_empty() && trimmed.starts_with("Side:") {
            continue;
        }
        paragraph.push(trimmed);
    }
    let joined = paragraph.join(" ");
    // Strip rustdoc intra-doc link targets like [`Foo`](crate::Foo) -> `Foo`, keeping full
    // first-paragraph Markdown intact so inline code spans are never cut mid-span.
    strip_intra_doc_links(&joined)
}

fn strip_intra_doc_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'['
            && let Some(close_bracket) = text[i + 1..].find(']')
        {
            let label_start = i + 1;
            let label_end = label_start + close_bracket;
            let after_bracket = label_end + 1;
            let label = &text[label_start..label_end];
            if after_bracket < bytes.len()
                && bytes[after_bracket] == b'('
                && let Some(close_paren) = text[after_bracket + 1..].find(')')
            {
                let target = &text[after_bracket + 1..after_bracket + 1 + close_paren];
                if !target.starts_with("http://")
                    && !target.starts_with("https://")
                    && !target.ends_with(".md")
                {
                    out.push_str(label);
                    i = after_bracket + 1 + close_paren + 1;
                    continue;
                }
            }
            // Also unwrap bare rustdoc shorthand [`Foo`] -> `Foo`.
            if label.starts_with('`') && label.ends_with('`') && label.len() >= 2 {
                out.push_str(label);
                i = after_bracket;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn escape_cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn checkout_renders_hooks_parts_and_host_methods_without_writing_source() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join("docs/modding/src/reference/api-reference.md");
        assert!(
            !path.exists(),
            "generated book pages must not be checked in"
        );
        let page = super::generate(&root).unwrap();
        assert!(page.starts_with("# Mod API reference\n"));
        assert!(page.contains("## Hooks (`Hooks` trait)"));
        assert!(page.contains("## Part hooks"));
        assert!(page.contains("### `BlockPart`"));
        assert!(page.contains("## Host methods (`Host`, `ServerHost`, `ClientHost`)"));
        assert!(page.contains("### Shared (`Host`)"));
        assert!(page.contains("### Server (`ServerHost`)"));
        assert!(page.contains("### Client (`ClientHost`)"));
        assert!(
            !page.contains("__private"),
            "private items must not appear in the generated API reference"
        );
        // Catalog-backed trait hooks appear under Hooks, while native render targets do not.
        let hooks_section = page
            .split("## Hooks (`Hooks` trait)")
            .nth(1)
            .and_then(|rest| rest.split("## Part hooks").next())
            .expect("Hooks section present");
        assert!(
            hooks_section.contains("| `anvil_update` |"),
            "expected catalog trait hook row in Hooks table"
        );
        assert!(
            !hooks_section.contains("| `world_drawn` |"),
            "native render targets must not be listed in the Hooks trait table"
        );
        // Event-only non-game hooks (e.g. get_enchantment_level) take no Host parameter.
        let enchant_row = hooks_section
            .lines()
            .find(|line| line.starts_with("| `get_enchantment_level` |"))
            .expect("get_enchantment_level row present");
        assert!(
            enchant_row.contains("| — |"),
            "expected event-only hook get_enchantment_level to document `—` for Host, got: {enchant_row}"
        );
        // MobEffectPart::apply_effect_tick uses the declaration-only effect target and takes &dyn Host.
        let effect_section = page
            .split("### `MobEffectPart`")
            .nth(1)
            .and_then(|rest| rest.split("\n### ").next())
            .expect("MobEffectPart section present");
        let effect_row = effect_section
            .lines()
            .find(|line| line.starts_with("| `apply_effect_tick` |"))
            .expect("apply_effect_tick row present");
        assert!(
            effect_row.contains("| `&dyn Host` |"),
            "expected MobEffectPart::apply_effect_tick to document `&dyn Host`, got: {effect_row}"
        );
        // Host summaries preserve their full first-paragraph Markdown without cutting inline code.
        let host_section = page
            .split("## Host methods (`Host`, `ServerHost`, `ClientHost`)")
            .nth(1)
            .expect("Host methods section present");
        let host_rows: Vec<_> = host_section
            .lines()
            .filter(|line| line.starts_with("| `"))
            .collect();
        assert!(!host_rows.is_empty(), "expected host method rows");
        for row in host_rows {
            assert_eq!(
                row.matches('`').count() % 2,
                0,
                "inline code spans in a host summary must stay balanced: {row}"
            );
        }
        assert!(!path.exists(), "rendering must not write into book sources");
    }

    #[test]
    fn strips_rustdoc_intra_links_but_preserves_http_links() {
        assert_eq!(
            super::strip_intra_doc_links("See [`Host`](crate::Host) and [`ItemStack`]."),
            "See `Host` and `ItemStack`."
        );
        assert_eq!(
            super::strip_intra_doc_links("Read [docs](https://playsunrise.net/docs)."),
            "Read [docs](https://playsunrise.net/docs)."
        );
    }
}
