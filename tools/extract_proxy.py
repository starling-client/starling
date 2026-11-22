#!/usr/bin/env python3
"""One-time move of the shared download proxy rules and tests out of port-pack."""

from pathlib import Path

CRATES = Path(__file__).resolve().parents[1] / "crates"
PORT_PACK = CRATES / "sunrise-port-pack/src/net.rs"
PROXY = CRATES / "sunrise-proxy/src/lib.rs"


def main():
    net = PORT_PACK.read_text(encoding="utf-8")
    _, shared = net.split("use std::collections::HashMap;", 1)
    rules, rest = ("use std::collections::HashMap;" + shared).split("/// An agent for downloading", 1)
    downloads, rest = ("/// An agent for downloading" + rest).split("/// The agent `url` is fetched with", 1)
    rules = rules.replace("fn spec_for(", "pub fn proxy_spec_for(")
    downloads = downloads.replace("spec_for(", "proxy_spec_for(")

    PROXY.parent.mkdir(parents=True, exist_ok=True)
    with PROXY.open("x", encoding="utf-8") as out:
        out.write(
            "//! Shared HTTP proxy selection and connection pools for Sunrise downloads.\n\n"
            + rules + "/// The agent `url` is fetched with" + rest
        )
    PORT_PACK.write_text(
        "//! Port-pack's download agents and redirect checks.\n\n"
        "use sunrise_proxy::proxy_spec_for;\n"
        "pub use sunrise_proxy::{direct, env_for, host_of, keep, parse, redacted, scheme_of, set_proxy};\n\n"
        + downloads.rstrip() + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
