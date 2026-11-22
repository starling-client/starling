//! The proxy and agent helpers the launcher's pages use (Horizon's `port_pack::net` as the sync base had it, MIT):
//! Sunrise's port pack keeps its own private now, and its launcher moved to another repository.
//! The agents port-pack's downloads go through: the jars of an imported modpack come from the same
//! places as everything else, so they take the same proxy — and leave the same hosts alone.
//!
//! The launcher hands its own setting over (`set_proxy`, from `proxy.rs`); a run of the
//! `sunrise-port-pack` command reads `HTTPS_PROXY`, `HTTP_PROXY` and `ALL_PROXY` instead (`env_for`),
//! taking the one that matches the address's scheme the way the launcher does. Either way a host in
//! `NO_PROXY`, or on this computer, is reached directly: a host that only answers without the proxy
//! would otherwise fail here while the launcher's own downloads worked.
//!
//! Which hosts those are is decided here and nowhere else. The launcher asks `direct`, `env_for`,
//! `host_of`, `scheme_of` and `parse` for its own traffic, and keeps its agents here too
//! (`keep`), so the two agree whatever the player set — and there is one copy of the rules to
//! change when they need it.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

/// What the launcher set, when it did.
static PROXY: Mutex<Option<String>> = Mutex::new(None);

/// Agents kept for reuse, named by the part that asked and the proxy they use: a fresh `Agent`
/// starts a new connection pool, so building one per request costs a handshake per file.
static KEPT: Mutex<Option<HashMap<String, ureq::Agent>>> = Mutex::new(None);

/// The proxy to use from now on: the launcher's setting, or None to fall back to the environment.
/// The agents built so far are dropped when it changes, since they hold the address it replaces.
pub fn set_proxy(spec: Option<String>) {
    let spec = spec.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
    let mut proxy = lock(&PROXY);
    if *proxy == spec {
        return;
    }
    *proxy = spec;
    drop(proxy);
    *lock(&KEPT) = None;
}

/// The proxy address `url` is fetched through: what the launcher set, else the environment's for
/// its scheme, unless `url` is a host that must be reached directly.
fn spec_for(url: &str) -> Option<String> {
    if direct(url) {
        return None;
    }
    lock(&PROXY)
        .clone()
        .or_else(|| env_for(scheme_of(url).unwrap_or("https")))
}

/// An agent for downloading `url`: through the proxy when one is set, unless `url` is a host that
/// must be reached directly.
pub fn agent(url: &str) -> ureq::Agent {
    keep("port-pack", spec_for(url).as_deref(), |proxy| {
        let builder = ureq::AgentBuilder::new();
        match proxy {
            Some(proxy) => builder.proxy(proxy).build(),
            None => builder.build(),
        }
    })
}

/// GET from allowed HTTPS hosts, validating every redirect before contacting it.
pub(crate) fn get_from_hosts(url: &str, hosts: &[&str]) -> Result<Option<ureq::Response>, String> {
    follow(url, hosts, |address| {
        let agent = keep(
            "port-pack-no-redirect",
            spec_for(address).as_deref(),
            |proxy| {
                let builder = ureq::AgentBuilder::new().redirects(0);
                match proxy {
                    Some(proxy) => builder.proxy(proxy).build(),
                    None => builder.build(),
                }
            },
        );
        match agent
            .get(address)
            .set(
                "User-Agent",
                "starling-client/starling (github.com/starling-client/starling)",
            )
            .call()
        {
            Ok(response) => Ok(Some(response)),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(error) => Err(format!("{address}: {error}")),
        }
    })
}

fn follow(
    url: &str,
    hosts: &[&str],
    mut request: impl FnMut(&str) -> Result<Option<ureq::Response>, String>,
) -> Result<Option<ureq::Response>, String> {
    let mut current = checked_url(url, hosts)?;
    for _ in 0..=5 {
        let Some(response) = request(current.as_str())? else {
            return Ok(None);
        };
        if !(300..400).contains(&response.status()) {
            return Ok(Some(response));
        }
        let location = response
            .header("Location")
            .ok_or("Redirect without Location")?;
        current = checked_url(
            current.join(location).map_err(|e| e.to_string())?.as_str(),
            hosts,
        )?;
    }
    Err("Too many redirects".into())
}

fn checked_url(address: &str, hosts: &[&str]) -> Result<url::Url, String> {
    let url = url::Url::parse(address).map_err(|e| e.to_string())?;
    if url.scheme() != "https"
        || !url.host_str().is_some_and(|host| hosts.contains(&host))
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Refused download URL outside allowed HTTPS hosts".into());
    }
    Ok(url)
}

/// The agent `url` is fetched with, built by `build` and then kept for the next request that names
/// the same `slot` and proxy: a fresh `Agent` starts a new connection pool, so building one per
/// request is a handshake per file, and a modpack's jars are fetched one after another.
pub fn keep(
    slot: &str,
    spec: Option<&str>,
    build: impl FnOnce(Option<ureq::Proxy>) -> ureq::Agent,
) -> ureq::Agent {
    let proxy = spec.and_then(parse);
    // An address that isn't a proxy is used as none, and remembered as none.
    let key = format!(
        "{slot} {}",
        if proxy.is_some() {
            spec.unwrap_or_default()
        } else {
            ""
        }
    );
    if let Some(agent) = lock(&KEPT).as_ref().and_then(|kept| kept.get(&key)) {
        return agent.clone();
    }
    let agent = build(proxy);
    lock(&KEPT)
        .get_or_insert_with(HashMap::new)
        .insert(key, agent.clone());
    agent
}

/// `spec` as a proxy, when it is one. An address that can't be used is said once: a download left
/// to go direct would otherwise fail with nothing pointing at the proxy.
pub fn parse(spec: &str) -> Option<ureq::Proxy> {
    // `socks5h` is curl's name for SOCKS5 with the proxy resolving host names. `ureq` has only the
    // name `socks5`, which already sends it host names to resolve.
    let address = match spec.strip_prefix("socks5h://") {
        Some(rest) => format!("socks5://{rest}"),
        None => spec.to_owned(),
    };
    match ureq::Proxy::new(&address) {
        Ok(proxy) => Some(proxy),
        Err(e) => {
            say(&format!(
                "The proxy address {} can't be used ({e}); downloads go without one.",
                redacted(spec)
            ));
            None
        }
    }
}

/// `spec` as it may be shown: a user name and password in it become `***`. An address that can't
/// be used is printed to stderr and shown in the launcher, and a proxy's credentials don't belong in
/// either. Everything up to the last `@` is the credentials, so a password with an unescaped `/` or
/// `@` is hidden too.
pub fn redacted(spec: &str) -> String {
    let (scheme, rest) = match spec.split_once("://") {
        Some((scheme, rest)) => (format!("{scheme}://"), rest),
        None => (String::new(), spec),
    };
    match rest.rfind('@') {
        Some(at) => format!("{scheme}***@{}", &rest[at + 1..]),
        None => spec.to_owned(),
    }
}

/// Whether `url` is reached without the proxy: a local address, or a host `NO_PROXY` names. The
/// launcher asks this of its own requests, so both reach the same hosts directly.
pub fn direct(url: &str) -> bool {
    host_of(url).is_some_and(|host| local(host) || excluded(host, &no_proxy()))
}

/// `HTTPS_PROXY` (`HTTP_PROXY` for an http address), then `ALL_PROXY`, in the upper-case spelling
/// first and the lower-case one after, as the tools that write them do.
pub fn env_for(scheme: &str) -> Option<String> {
    let names: &[&str] = if scheme.eq_ignore_ascii_case("http") {
        &["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        &["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    };
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    })
}

/// `NO_PROXY`'s list: the hosts reached directly whatever the proxy is.
fn no_proxy() -> String {
    ["NO_PROXY", "no_proxy"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .unwrap_or_default()
}

/// The host part of a URL (`https://user@host:port/path` is `host`).
pub fn host_of(url: &str) -> Option<&str> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['/', '?', '#']).next()?;
    let rest = rest.rsplit_once('@').map_or(rest, |(_, host)| host);
    Some(match rest.rfind(':') {
        // A port follows the last colon; an IPv6 literal keeps the colons inside its brackets.
        Some(at) if !rest[at..].contains(']') => &rest[..at],
        _ => rest,
    })
}

/// `https` in `https://…`, or None for anything that isn't an absolute address.
pub fn scheme_of(url: &str) -> Option<&str> {
    url.split_once("://").map(|(scheme, _)| scheme)
}

/// Whether `host` is this computer: the game's own API and a server on the same machine.
fn local(host: &str) -> bool {
    let host = bare(host);
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "::1"
        || host == "0.0.0.0"
        || is_loopback_v4(&host)
}

/// A dotted IPv4 address in `127.0.0.0/8`; a DNS name that merely starts with `127.` is not one.
fn is_loopback_v4(host: &str) -> bool {
    host.starts_with("127.")
        && host
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `host` is in `NO_PROXY`: `*`, `host`, `.domain`, `*.domain` and `host:port` entries.
fn excluded(host: &str, no_proxy: &str) -> bool {
    let host = bare(host);
    no_proxy
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .any(|entry| {
            if entry == "*" {
                return true;
            }
            let entry = entry.trim_start_matches('*').trim_start_matches('.');
            // A port follows the last colon of a bracketed entry, or of a host with no colon
            // before it; an unbracketed IPv6 literal keeps its colons.
            let entry = match entry.rfind(':') {
                Some(at)
                    if !entry[at..].contains(']')
                        && (entry[..at].ends_with(']') || !entry[..at].contains(':')) =>
                {
                    &entry[..at]
                }
                _ => entry,
            };
            let entry = bare(entry);
            !entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}")))
        })
}

/// A host without the brackets an IPv6 literal wears, lower case.
fn bare(host: &str) -> String {
    host.trim().trim_matches(['[', ']']).to_ascii_lowercase()
}

/// Said once, however many downloads follow: a wrong address is one thing to report, not one
/// report per file.
fn say(what: &str) {
    static SAID: AtomicBool = AtomicBool::new(false);
    if !SAID.swap(true, Ordering::Relaxed) {
        eprintln!("{what}");
    }
}

/// The lock, even after a thread panicked while holding it: an address lost to a panic is worse
/// than an old one kept.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
