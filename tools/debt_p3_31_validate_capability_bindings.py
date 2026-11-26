#!/usr/bin/env python3
"""Move native capability binding inspection from the four adapters to their registrar."""

from pathlib import Path

PARTS = Path(__file__).resolve().parent.parent / "crates/sunrise-mod-api/src/parts"


def replace_once(source: str, old: str, new: str) -> str:
    if source.count(old) != 1:
        raise ValueError(f"expected one occurrence of {old!r}")
    return source.replace(old, new)


def main() -> None:
    adapter = """            {
                let handler = mod_vocab::__private::parts_installation::capability_provider_handler(
                    &provider,
                );
                (provider, handler)
            },"""
    changes = {}
    for module in ("blocks", "block_entities", "items", "entities"):
        path = PARTS / f"{module}.rs"
        changes[path] = replace_once(path.read_text(), adapter, "            provider,")

    path = PARTS / "capabilities.rs"
    source = replace_once(
        path.read_text(),
        """    pub(crate) fn capability<P>(
        &mut self,
        table: fn(&mut Parts) -> &mut Capabilities<P>,
        shares_names: Option<fn(&Parts) -> &Capabilities<P>>,""",
        """    pub(crate) fn capability(
        &mut self,
        table: fn(&mut Parts) -> &mut Capabilities<NativeCapabilityProvider>,
        shares_names: Option<fn(&Parts) -> &Capabilities<NativeCapabilityProvider>>,""",
    )
    source = replace_once(
        source,
        """        (provider, handler): (P, impl Into<Arc<str>>),
    ) {
        let handler = handler.into();""",
        """        provider: NativeCapabilityProvider,
    ) {
        let handler: Arc<str> = mod_vocab::__private::parts_installation::capability_provider_handler(
            &provider,
        )
        .into();""",
    )
    changes[path] = replace_once(source, "&Capabilities<P>|", "&Capabilities<NativeCapabilityProvider>|")
    for path, source in changes.items():
        path.write_text(source)


if __name__ == "__main__":
    main()
