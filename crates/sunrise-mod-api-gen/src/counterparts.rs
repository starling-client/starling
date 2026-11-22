//! `docs/modding/src/reference/neoforge-counterparts.md` from each declaration's `neoforge` keys.
//!
//! A counterpart belongs to the hook it names, so two PRs that add different hooks never edit the
//! same page. The book preprocessor uses `cargo run -p xtask -- mod-api counterparts` to render the
//! chapter in memory; no generated Markdown is committed. Reading the declarations is the catalog
//! loader's job: this module only groups the keys it already carries.
use crate::catalog::{Catalog, Side};
use std::path::Path;

/// A hook's row: its name, side and the cell beside them.
struct Row {
    name: String,
    side: &'static str,
    cell: String,
}

/// Renders the page from the declarations, in hook-name order.
pub fn render(catalog: &Catalog) -> String {
    let mut rows = catalog
        .hooks
        .iter()
        .map(|hook| Row {
            name: hook.name.clone(),
            side: side(hook.side),
            cell: hook.counterparts.neoforge.join(", "),
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    let total = rows.len();
    let mut mapped = Vec::new();
    let mut none = Vec::new();
    for row in rows {
        // A declaration with no `neoforge` key is counted, but it belongs to neither table: the
        // page lists declarations, and `none (...)` is the way to state a reason.
        if row.cell.is_empty() {
            continue;
        }
        if names_none(&row.cell) {
            none.push(Row {
                cell: reason(&row.cell),
                ..row
            });
        } else {
            mapped.push(Row {
                cell: format!("`{}`", row.cell),
                ..row
            });
        }
    }
    format!(
        "# NeoForge counterparts\n\n\
Each hook's `neoforge = ...` key in `crates/sunrise-mod-vocab/src/hooks/` names the NeoForge 26.3 event or method it\n\
matches, since a counterpart's name and contract are the rule for a hook. This page lists the keys as declared.\n\
The book build renders it directly from those declarations; there is no generated Markdown snapshot to update.\n\
A key names the event class (`PlayerEvent.Clone`) or runtime callback (`IPayloadHandler.handle`);\n\
notes about cancelling or the event bus belong in the hook's `///` doc, not in the key.\n\
\n\
{total} hooks: {mapped_count} name a counterpart, {none_count} say they have none.\n\
This is a declaration reference, not an independent verification of every mapping. Registration events and the\n\
callbacks they register are different moments: packet receipt names `IPayloadHandler.handle`, and per-draw item\n\
decoration names `IItemDecorator.render`, not their registration events. A `none` explains why no event or method\n\
matches that hook's runtime contract.\n\
\n\
## Hooks with a counterpart\n\
\n\
{}\n\
\n\
## Hooks that declare no counterpart\n\
\n\
A hook with no NeoForge counterpart is judged on its merits like any other. The note is the key's own reason.\n\
\n\
{}\n",
        table(&mapped, "NeoForge"),
        table(&none, "Reason"),
        mapped_count = mapped.len(),
        none_count = none.len(),
    )
}

/// Reads the checkout's declarations and renders the page.
///
/// The declared names alone make this page, so it reads declarations without resolving their
/// targets into game sources. Game-side generation still resolves and validates native targets.
pub fn generate(root: &Path) -> Result<String, String> {
    let catalog = Catalog::load(root).map_err(|error| error.to_string())?;
    Ok(render(&catalog))
}

fn side(side: Side) -> &'static str {
    match side {
        Side::Server => "server",
        Side::Client => "client",
        Side::Both => "both",
    }
}

/// Whether a declared counterpart says the hook has none, rather than naming one.
fn names_none(declared: &str) -> bool {
    declared
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("none"))
}

/// The reason inside `none (...)`, or the whole declaration for a bare `none`.
fn reason(declared: &str) -> String {
    let stripped = declared
        .strip_prefix("none")
        .unwrap_or(declared)
        .trim_start();
    let stripped = stripped.strip_prefix('(').unwrap_or(stripped);
    let stripped = stripped.strip_suffix(')').unwrap_or(stripped);
    let reason = stripped.trim();
    if reason.is_empty() {
        "none".to_owned()
    } else {
        reason.to_owned()
    }
}

fn table(rows: &[Row], head: &str) -> String {
    let mut table = format!("| Hook | Side | {head} |\n| --- | --- | --- |");
    for row in rows {
        table.push_str(&format!(
            "\n| `{}` | {} | {} |",
            row.name, row.side, row.cell
        ));
    }
    table
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    const DECLARATIONS: &str = r###"
        pub trait Hooks {
            /// Beta.
            #[hook(
                side = "both",
                contract = "listen",
                target = "example",
                neoforge = ["FirstEvent", "SecondEvent"]
            )]
            fn beta();
            /// Alpha.
            #[hook(side = "client", contract = "listen", target = "example", neoforge = "GuiLayer.render")]
            fn alpha();
            /// No counterpart key at all.
            #[hook(side = "server", contract = "listen", target = "example")]
            fn gamma();
            /// A stated reason.
            #[hook(
                side = "client",
                contract = "listen",
                target = "example",
                neoforge = "none (runtime callback, not registration)"
            )]
            fn delta();
        }
    "###;

    fn render(source: &str) -> String {
        let catalog = crate::catalog::Catalog::parse(&[("test.rs".into(), source.into())]).unwrap();
        super::render(&catalog)
    }

    #[test]
    fn the_page_layout_is_exact() {
        let source = r###"
            pub trait Hooks {
                /// Alpha.
                #[hook(side = "client", contract = "listen", target = "example", neoforge = "GuiLayer.render")]
                fn alpha();
                /// Delta.
                #[hook(
                    side = "client",
                    contract = "listen",
                    target = "example",
                    neoforge = "none (runtime callback)"
                )]
                fn delta();
                /// Gamma.
                #[hook(side = "server", contract = "listen", target = "example")]
                fn gamma();
            }
        "###;
        assert_eq!(
            render(source),
            r#"# NeoForge counterparts

Each hook's `neoforge = ...` key in `crates/sunrise-mod-vocab/src/hooks/` names the NeoForge 26.3 event or method it
matches, since a counterpart's name and contract are the rule for a hook. This page lists the keys as declared.
The book build renders it directly from those declarations; there is no generated Markdown snapshot to update.
A key names the event class (`PlayerEvent.Clone`) or runtime callback (`IPayloadHandler.handle`);
notes about cancelling or the event bus belong in the hook's `///` doc, not in the key.

3 hooks: 1 name a counterpart, 1 say they have none.
This is a declaration reference, not an independent verification of every mapping. Registration events and the
callbacks they register are different moments: packet receipt names `IPayloadHandler.handle`, and per-draw item
decoration names `IItemDecorator.render`, not their registration events. A `none` explains why no event or method
matches that hook's runtime contract.

## Hooks with a counterpart

| Hook | Side | NeoForge |
| --- | --- | --- |
| `alpha` | client | `GuiLayer.render` |

## Hooks that declare no counterpart

A hook with no NeoForge counterpart is judged on its merits like any other. The note is the key's own reason.

| Hook | Side | Reason |
| --- | --- | --- |
| `delta` | client | runtime callback |
"#
        );
    }

    #[test]
    fn rows_are_sorted_by_hook_name_and_a_missing_key_is_counted_but_not_listed() {
        let page = render(DECLARATIONS);
        assert!(page.starts_with("# NeoForge counterparts\n\n"));
        assert!(page.contains("4 hooks: 2 name a counterpart, 1 say they have none."));
        let alpha = page
            .find("| `alpha` | client | `GuiLayer.render` |")
            .unwrap();
        let beta = page
            .find("| `beta` | both | `FirstEvent, SecondEvent` |")
            .unwrap();
        assert!(alpha < beta, "declaration order is not the page's order");
        assert!(page.contains("| `delta` | client | runtime callback, not registration |"));
        assert!(!page.contains("| `gamma` |"));
        assert!(
            page.find("## Hooks with a counterpart").unwrap()
                < page.find("## Hooks that declare no counterpart").unwrap()
        );
    }

    #[test]
    fn a_bare_none_says_none_and_a_none_list_keeps_the_rest_of_the_text() {
        let source = r###"
            pub trait Hooks {
                /// Bare.
                #[hook(side = "server", contract = "listen", target = "example", neoforge = "none")]
                fn bare();
                /// Listed.
                #[hook(
                    side = "server",
                    contract = "listen",
                    target = "example",
                    neoforge = ["none (a), b"]
                )]
                fn listed();
            }
        "###;
        let page = render(source);
        assert!(page.contains("| `bare` | server | none |"));
        assert!(page.contains("| `listed` | server | a), b |"));
        assert!(page.contains("2 hooks: 0 name a counterpart, 2 say they have none."));
    }

    #[test]
    fn comments_and_notes_holding_a_hook_are_not_declarations() {
        let source = r###"
            pub trait Hooks {
                // #[hook(side = "server", contract = "listen", neoforge = "FakeEvent")] fn fake();
                /* #[hook(side = "server", contract = "listen", neoforge = "OtherFake")] fn fake(); */
                /// Draw.
                #[notes = r#"An example: #[hook(side = "server", neoforge = "ThirdFake")] fn fake();"#]
                #[hook(side = "client", contract = "listen", target = "example", neoforge = "GuiLayer.render")]
                fn draw();
            }
        "###;
        let page = render(source);
        assert!(page.contains("| `draw` | client | `GuiLayer.render` |"));
        assert!(!page.contains("FakeEvent"));
        assert!(!page.contains("OtherFake"));
        assert!(!page.contains("ThirdFake"));
        assert!(!page.contains("`fake`"));
        assert!(page.contains("1 hooks: 1 name a counterpart, 0 say they have none."));
    }

    #[test]
    fn comments_inside_the_attribute_do_not_hide_or_add_a_key() {
        let source = r###"
            pub trait Hooks {
                /// Draw.
                #[hook(
                    side = "client", // a comment with ] and neoforge = "FakeEvent"
                    /* a block comment with ] and fn fake() /* nested */ still a comment */
                    contract = "listen",
                    target = "example",
                    neoforge = "GuiLayer.render"
                )]
                fn draw();
            }
        "###;
        let page = render(source);
        assert!(page.contains("| `draw` | client | `GuiLayer.render` |"));
        assert!(!page.contains("FakeEvent"));
    }

    #[test]
    fn the_checkout_renders_from_declarations_without_writing_a_source_page() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = root.join("docs/modding/src/reference/neoforge-counterparts.md");
        assert!(
            !path.exists(),
            "generated book pages must not be checked in"
        );
        let page = super::generate(&root).unwrap();
        assert!(page.contains("| `item_decorations` | client | `IItemDecorator.render` |"));
        assert!(page.contains("## Hooks that declare no counterpart"));
        assert!(!path.exists(), "rendering must not write into book sources");
    }
}
