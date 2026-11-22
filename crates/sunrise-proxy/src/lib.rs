//! Shared HTTP proxy selection and connection pools for Sunrise downloads.
//!
//! Local hosts and hosts in `NO_PROXY` go direct. Other URLs use `HTTPS_PROXY` or `HTTP_PROXY`
//! for the URL's scheme, then `ALL_PROXY`.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fmt;
use std::net::Ipv6Addr;
use std::num::NonZeroU16;
use std::sync::{LazyLock, Mutex};
use url::Url;

type Agents = HashMap<AgentKey, ureq::Agent>;

/// A fresh agent starts a new connection pool, so reuse agents with the same settings.
static KEPT: LazyLock<Mutex<Agents>> = LazyLock::new(Mutex::default);

#[derive(PartialEq, Eq, Hash)]
struct AgentKey {
    slot: String,
    proxy: Option<String>,
}

/// A configuration error that can be reported without showing the proxy address or credentials.
#[derive(Debug)]
pub struct ProxyError {
    setting: &'static str,
    kind: InvalidProxy,
}

#[derive(Debug, PartialEq, Eq)]
enum InvalidProxy {
    Authority,
    Scheme,
    Encoding,
}

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match self.kind {
            InvalidProxy::Authority => "invalid proxy authority",
            InvalidProxy::Scheme => "unsupported proxy scheme",
            InvalidProxy::Encoding => "proxy value is not valid Unicode",
        };
        write!(f, "{}: {reason}", self.setting)
    }
}

impl std::error::Error for ProxyError {}

/// A kept agent for `url`, using the environment unless the destination is reached directly.
/// An invalid selected proxy is an error, not a direct route.
/// Each `slot` names one builder configuration. `configure` only runs when the slot has no agent
/// for the selected proxy, and should not set a proxy itself.
pub fn agent(
    slot: &str,
    url: &str,
    configure: impl FnOnce(ureq::AgentBuilder) -> ureq::AgentBuilder,
) -> Result<ureq::Agent, ProxyError> {
    agent_for(slot, Url::parse(url).ok().as_ref(), configure)
}

fn agent_for(
    slot: &str,
    url: Option<&Url>,
    configure: impl FnOnce(ureq::AgentBuilder) -> ureq::AgentBuilder,
) -> Result<ureq::Agent, ProxyError> {
    let selected = if url.is_some_and(|url| bypasses_proxy(url, &no_proxy())) {
        None
    } else {
        let scheme = url.map_or("https", Url::scheme);
        env_proxy(scheme, |name| std::env::var_os(name))?
    };
    let mut agents = KEPT.lock().unwrap_or_else(|e| e.into_inner());
    keep(&mut agents, slot, selected, |proxy| {
        let builder = configure(ureq::AgentBuilder::new()).try_proxy_from_env(false);
        match proxy {
            Some(proxy) => builder.proxy(proxy).build(),
            None => builder.build(),
        }
    })
}

/// An HTTP failure or a request that could not be sent or followed.
#[derive(Debug)]
pub enum GetError {
    Status { url: String, code: u16 },
    Transport { url: String, kind: ureq::ErrorKind },
    Request(String),
}

impl GetError {
    /// The server's status code, if the request failed with an HTTP error status.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { code, .. } => Some(*code),
            Self::Transport { .. } | Self::Request(_) => None,
        }
    }
}

impl fmt::Display for GetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status { url, code } => write!(f, "{url}: HTTP {code}"),
            Self::Transport { url, kind } => write!(f, "{url}: {kind}"),
            Self::Request(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for GetError {}

/// An optional policy for the initial URL and each redirect destination, before any request.
/// Refusal messages must not include credentials or other sensitive URL data.
pub type UrlCheck<'a> = dyn Fn(&Url) -> Result<(), String> + 'a;

/// GET with a fresh proxy decision per hop, including `NO_PROXY`. Only `Accept` and
/// `User-Agent` from `headers` are carried to a redirect destination. Only 301, 302, 303,
/// 307 and 308 are followed; other 3xx responses are returned unchanged.
pub fn get(
    url: &str,
    headers: &[(&str, &str)],
    check: Option<&UrlCheck<'_>>,
) -> Result<ureq::Response, GetError> {
    follow(url, check, |current, first| {
        // This slot is separate from agents that are allowed to follow redirects internally.
        let agent = agent_for("get-no-redirect", Some(current), |builder| {
            builder.redirects(0)
        })
        .map_err(|e| GetError::Request(e.to_string()))?;
        let mut request = agent.request_url("GET", current);
        for &(name, value) in headers {
            if first
                || ["accept", "user-agent"]
                    .iter()
                    .any(|h| name.eq_ignore_ascii_case(h))
            {
                request = request.set(name, value);
            }
        }
        request.call().map_err(|error| {
            // Don't retain ureq's error: its URL, response and nested messages can contain secrets.
            let mut url = current.clone();
            url.set_query(None);
            url.set_fragment(None);
            let _ = url.set_password(None);
            let _ = url.set_username("");
            let url = url.into();
            match error {
                ureq::Error::Status(code, _) => GetError::Status { url, code },
                ureq::Error::Transport(error) => GetError::Transport {
                    url,
                    kind: error.kind(),
                },
            }
        })
    })
}

fn follow(
    url: &str,
    check: Option<&UrlCheck<'_>>,
    mut request: impl FnMut(&Url, bool) -> Result<ureq::Response, GetError>,
) -> Result<ureq::Response, GetError> {
    const MAX_REDIRECTS: usize = 5;
    let mut current = Url::parse(url).map_err(|e| GetError::Request(e.to_string()))?;
    for hop in 0..=MAX_REDIRECTS {
        if let Some(check) = check {
            check(&current).map_err(GetError::Request)?;
        }
        if !matches!(current.scheme(), "http" | "https") {
            return Err(GetError::Request("Refused non-HTTP URL".into()));
        }
        let response = request(&current, hop == 0)?;
        if !matches!(response.status(), 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        if hop == MAX_REDIRECTS {
            break;
        }
        let location = response
            .header("Location")
            .ok_or_else(|| GetError::Request("Redirect without Location".into()))?;
        current = current
            .join(location)
            .map_err(|e| GetError::Request(e.to_string()))?;
    }
    Err(GetError::Request("Too many redirects".into()))
}

/// An agent built by `build` and kept for the next request with the same `slot` and proxy.
/// Building a fresh agent every time would give each request a new connection pool.
/// Only `None` requests a direct agent; an invalid `spec` returns an error without calling `build`.
fn keep(
    agents: &mut Agents,
    slot: &str,
    selected: Option<(&'static str, String)>,
    build: impl FnOnce(Option<ureq::Proxy>) -> ureq::Agent,
) -> Result<ureq::Agent, ProxyError> {
    let (setting, proxy) = selected.map_or(("", None), |(name, spec)| (name, Some(spec)));
    let key = AgentKey {
        slot: slot.into(),
        proxy,
    };
    match agents.entry(key) {
        std::collections::hash_map::Entry::Occupied(entry) => Ok(entry.get().clone()),
        std::collections::hash_map::Entry::Vacant(entry) => {
            let proxy = entry
                .key()
                .proxy
                .as_deref()
                .map(parse_proxy)
                .transpose()
                .map_err(|kind| ProxyError { setting, kind })?;
            Ok(entry.insert(build(proxy)).clone())
        }
    }
}

fn parse_proxy(spec: &str) -> Result<ureq::Proxy, InvalidProxy> {
    let spec = spec.trim_end_matches('/');
    let (scheme, authority) = spec.split_once("://").unwrap_or(("http", spec));
    let scheme = match scheme {
        "http" | "socks4" | "socks4a" | "socks" | "socks5" => scheme,
        // ureq's SOCKS5 already sends the proxy host names to resolve.
        "socks5h" => "socks5",
        scheme if valid_scheme(scheme) => return Err(InvalidProxy::Scheme),
        _ => return Err(InvalidProxy::Authority),
    };
    // ureq splits the address at colons and silently defaults an unparseable port. Reject a
    // malformed authority (including IPv6 proxy endpoints, which that parser cannot represent).
    let address = authority
        .rsplit_once('@')
        .map_or(authority, |(_, addr)| addr);
    let (host, port) = address
        .split_once(':')
        .map_or((address, None), |(host, port)| (host, Some(port)));
    if host.is_empty()
        || host
            .chars()
            .any(|c| c.is_whitespace() || "[]/\\?#".contains(c))
        || authority.chars().any(char::is_control)
        || port.is_some_and(|p| {
            !p.bytes().all(|b| b.is_ascii_digit()) || p.parse::<NonZeroU16>().is_err()
        })
    {
        return Err(InvalidProxy::Authority);
    }
    ureq::Proxy::new(format!("{scheme}://{authority}")).map_err(|_| InvalidProxy::Authority)
}

fn valid_scheme(scheme: &str) -> bool {
    matches!(scheme.as_bytes(), [first, rest @ ..] if first.is_ascii_alphabetic()
        && rest.iter().all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(b)))
}

fn env_proxy(
    scheme: &str,
    mut read: impl FnMut(&str) -> Option<OsString>,
) -> Result<Option<(&'static str, String)>, ProxyError> {
    let names = if scheme.eq_ignore_ascii_case("http") {
        ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    };
    names
        .into_iter()
        .find_map(|name| {
            let value = read(name)?;
            let Some(value) = value.to_str() else {
                return Some(Err(ProxyError {
                    setting: name,
                    kind: InvalidProxy::Encoding,
                }));
            };
            let value = value.trim();
            (!value.is_empty()).then(|| Ok((name, value.to_owned())))
        })
        .transpose()
}

/// `NO_PROXY`'s list: the hosts reached directly whatever the proxy is.
fn no_proxy() -> String {
    ["NO_PROXY", "no_proxy"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .unwrap_or_default()
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

/// Whether the destination is local or in `NO_PROXY`: `*`, `host`, `.domain`, `*.domain` and
/// `host:port` entries (matching the effective port, including the scheme's default).
fn bypasses_proxy(url: &url::Url, no_proxy: &str) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    if local(host) {
        return true;
    }
    let host = bare(host);
    let port = url.port_or_known_default();
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
                    if !entry[at + 1..]
                        .parse::<u16>()
                        .is_ok_and(|required| Some(required) == port)
                    {
                        return false;
                    }
                    &entry[..at]
                }
                _ => entry,
            };
            let entry = bare(entry);
            !entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}")))
        })
}

/// A lower-case host name or canonical IP literal, without IPv6 brackets or a DNS root dot.
fn bare(host: &str) -> String {
    let host = host.trim();
    let host = host
        .strip_suffix('.')
        .unwrap_or(host)
        .trim_matches(['[', ']']);
    if host.contains(':')
        && let Ok(ip) = host.parse::<Ipv6Addr>()
    {
        return ip.to_string();
    }
    host.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn this_computer_is_never_proxied() {
        for host in [
            "localhost",
            "LOCALHOST",
            "[::1]",
            "::1",
            "127.0.0.1",
            "127.1.2.3",
        ] {
            assert!(local(host), "{host}");
        }
        for host in [
            "playsunrise.net",
            "api.minecraftservices.com",
            "10.0.0.1",
            "127.example.com",
            "127.0.0.1.example.com",
        ] {
            assert!(!local(host), "{host}");
        }
    }

    #[test]
    fn proxy_environment_prefers_scheme_over_all_and_uppercase_over_lowercase() {
        let mut values = HashMap::from([
            ("HTTP_PROXY", "http://upper.example"),
            ("http_proxy", "http://lower.example"),
            ("https_proxy", " socks5h://secure.example "),
            ("ALL_PROXY", "http://all.example"),
            ("all_proxy", "http://all-lower.example"),
        ]);
        let selected = |scheme, values: &HashMap<&str, &str>| {
            env_proxy(scheme, |name| values.get(name).map(OsString::from))
                .unwrap()
                .map(|(_, value)| value)
        };
        assert_eq!(
            selected("http", &values).as_deref(),
            Some("http://upper.example")
        );
        assert_eq!(
            selected("https", &values).as_deref(),
            Some("socks5h://secure.example")
        );
        values.insert("HTTPS_PROXY", "http://secure-upper.example");
        assert_eq!(
            selected("https", &values).as_deref(),
            Some("http://secure-upper.example")
        );
        values.insert("HTTP_PROXY", " ");
        assert_eq!(
            selected("http", &values).as_deref(),
            Some("http://lower.example")
        );
        values.remove("http_proxy");
        assert_eq!(
            selected("http", &values).as_deref(),
            Some("http://all.example")
        );
        values.remove("ALL_PROXY");
        assert_eq!(
            selected("http", &values).as_deref(),
            Some("http://all-lower.example")
        );
        values.remove("all_proxy");
        assert_eq!(selected("http", &values), None);
    }

    fn bypasses(url: &str, no_proxy: &str) -> bool {
        bypasses_proxy(&url::Url::parse(url).unwrap(), no_proxy)
    }

    #[test]
    fn no_proxy_lists_hosts_and_their_subdomains() {
        assert!(bypasses("https://anything.at.all", "*"));
        assert!(bypasses("https://cdn.modrinth.com", "modrinth.com"));
        assert!(bypasses("https://api.modrinth.com", "MODRINTH.com"));
        assert!(bypasses("https://piston-meta.mojang.com", "*.mojang.com"));
        assert!(bypasses(
            "https://wiki.corp.example",
            " .corp.example , other.host "
        ));
        assert!(!bypasses("https://notmodrinth.com", "modrinth.com"));
        assert!(!bypasses(
            "https://modrinth.com",
            " .corp.example , mojang.com "
        ));
        assert!(!bypasses("https://edge.forgecdn.net", "modrinth.com"));
        assert!(!bypasses("https://modrinth.com", ""));
        assert!(!bypasses("http://192.0.2.10/", "192.0.2.0/24"));
    }

    #[test]
    fn a_dns_root_dot_does_not_change_the_route() {
        assert!(bypasses("https://localhost./", ""));
        assert!(bypasses("https://tools.localhost./", ""));
        assert!(bypasses("https://cdn.example.org./", "example.org"));
        assert!(bypasses("https://cdn.example.org/", ".EXAMPLE.ORG."));
        assert!(bypasses(
            "https://cdn.example.org.:8443/",
            "example.org.:8443"
        ));
        assert!(!bypasses("https://cdn.example.org./", "example.org.:8443"));
        assert!(!bypasses("https://notexample.org./", "example.org."));
    }

    #[test]
    fn no_proxy_requires_the_specified_destination_port() {
        assert!(bypasses(
            "https://mirror.example:8443/",
            "mirror.example:8443"
        ));
        assert!(!bypasses("https://mirror.example/", "mirror.example:8443"));
        assert!(bypasses("https://mirror.example/", "mirror.example:443"));
        assert!(!bypasses("http://mirror.example/", "mirror.example:443"));
        assert!(bypasses("http://mirror.example/", "mirror.example:80"));
        assert!(!bypasses("http://mirror.example/", "mirror.example:http"));
        assert!(bypasses("https://mirror.example:8443/", "mirror.example"));
        assert!(!bypasses(
            r"https://other.example\@mirror.example:8443/",
            "mirror.example:8443"
        ));
    }

    #[test]
    fn no_proxy_matches_ipv6_literals_with_or_without_brackets() {
        let url = "http://[2001:0db8::1]:8080/";
        assert!(bypasses(url, "2001:db8::1"));
        assert!(bypasses(url, "[2001:db8::1]"));
        assert!(bypasses(url, "[2001:0db8::1]:8080"));
        assert!(!bypasses(url, "[2001:db8::1]:8081"));
        assert!(!bypasses(url, "2001:db8::2"));
    }

    #[test]
    fn a_selected_invalid_proxy_never_builds_a_direct_agent() {
        let mut agents = Agents::new();
        // An existing direct agent must not be reused when selection produces an invalid proxy.
        keep(&mut agents, "invalid", None, |_| {
            ureq::AgentBuilder::new().build()
        })
        .unwrap();
        for (spec, kind) in [
            (
                "http://proxy.example:dummy-password",
                InvalidProxy::Authority,
            ),
            ("http://proxy.example:65536", InvalidProxy::Authority),
            ("http://proxy.example:", InvalidProxy::Authority),
            ("http://:3128", InvalidProxy::Authority),
            ("http://proxy.example/path", InvalidProxy::Authority),
            (
                "dummy-user:dummy-password://with@delimiters@proxy.example",
                InvalidProxy::Authority,
            ),
            (
                "https://dummy-user:dummy-password@proxy.example",
                InvalidProxy::Scheme,
            ),
        ] {
            let fail = |_| panic!("an invalid proxy must not be used to build an agent");
            let error = keep(
                &mut agents,
                "invalid",
                Some(("HTTP_PROXY", spec.into())),
                fail,
            )
            .unwrap_err();
            assert_eq!(error.setting, "HTTP_PROXY");
            assert_eq!(error.kind, kind);
            let message = error.to_string();
            assert!(!message.contains("dummy-user"));
            assert!(!message.contains("dummy-password"));
        }
        for spec in [
            "http://127.0.0.1:7890",
            "127.0.0.1:7890",
            "dummy-user:dummy-password@proxy.example:3128",
            "socks5://127.0.0.1:7891",
        ] {
            assert!(parse_proxy(spec).is_ok());
        }
    }

    /// A stand-in SOCKS5 proxy: answers one request itself and returns the host it was asked for.
    fn socks5_proxy() -> (u16, std::thread::JoinHandle<String>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TIMEOUT)).unwrap();
            let mut greeting = [0; 2];
            stream.read_exact(&mut greeting).unwrap();
            assert_eq!(greeting[0], 5);
            let mut methods = vec![0; greeting[1].into()];
            stream.read_exact(&mut methods).unwrap();
            stream.write_all(&[5, 0]).unwrap();
            let mut request = [0; 5];
            stream.read_exact(&mut request).unwrap();
            assert_eq!(request[..4], [5, 1, 0, 3], "a CONNECT to a host name");
            let mut host = vec![0; request[4].into()];
            stream.read_exact(&mut host).unwrap();
            let mut target_port = [0; 2];
            stream.read_exact(&mut target_port).unwrap();
            stream.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).unwrap();
            let mut http = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while http.read_line(&mut line).unwrap() > 2 {
                line.clear();
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
            String::from_utf8(host).unwrap()
        });
        (port, server)
    }

    /// `.invalid` never resolves here (RFC 2606), so the answer can only have come through the
    /// proxy, which is handed the name to resolve.
    #[test]
    #[ignore = "daily"]
    fn a_request_goes_through_a_socks_proxy() {
        for scheme in ["socks5", "socks5h"] {
            let (port, server) = socks5_proxy();
            let proxy = parse_proxy(&format!("{scheme}://127.0.0.1:{port}")).unwrap();
            let agent = ureq::AgentBuilder::new()
                .timeout_connect(TIMEOUT)
                .timeout(TIMEOUT)
                .proxy(proxy)
                .build();
            let answer = agent.get("http://sunrise.invalid/a.jar").call().unwrap();
            assert_eq!(answer.into_string().unwrap(), "ok", "{scheme}");
            assert_eq!(server.join().unwrap(), "sunrise.invalid", "{scheme}");
        }
    }

    #[test]
    fn checks_each_redirect_before_requesting_it() {
        let check = |url: &Url| {
            (url.scheme() == "https"
                && matches!(
                    url.host_str(),
                    Some("github.com" | "release-assets.githubusercontent.com")
                ))
            .then_some(())
            .ok_or_else(|| "unlisted host".into())
        };
        let mut seen = Vec::new();
        let result = follow("https://github.com/file", Some(&check), |url, _| {
            seen.push(url.to_string());
            let reply = match seen.len() {
                1 => "HTTP/1.1 302 Found\r\nLocation: /release\r\n\r\n",
                2 => {
                    "HTTP/1.1 302 Found\r\nLocation: https://release-assets.githubusercontent.com/file\r\n\r\n"
                }
                3 => "HTTP/1.1 302 Found\r\nLocation: https://other.invalid/file\r\n\r\n",
                _ => "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
            };
            Ok(reply.parse().unwrap())
        });
        assert!(result.is_err());
        assert_eq!(
            seen,
            [
                "https://github.com/file",
                "https://github.com/release",
                "https://release-assets.githubusercontent.com/file"
            ]
        );
    }

    #[test]
    fn not_modified_does_not_follow_location() {
        let mut requests = 0;
        let response = follow("https://assets.invalid/file", None, |_, _| {
            requests += 1;
            Ok("HTTP/1.1 304 Not Modified\r\nLocation: /other\r\n\r\n"
                .parse()
                .unwrap())
        })
        .unwrap();
        assert_eq!(response.status(), 304);
        assert_eq!(requests, 1);
    }

    #[test]
    fn stops_after_five_redirects() {
        let mut requests = 0;
        let result = follow("http://mirror.invalid/file", None, |_, _| {
            requests += 1;
            Ok("HTTP/1.1 302 Found\r\nLocation: /file\r\n\r\n"
                .parse()
                .unwrap())
        });
        assert!(result.is_err());
        assert_eq!(requests, 6);
    }

    #[test]
    #[ignore = "daily"]
    fn redirects_select_the_proxy_for_each_destination() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        const LOCAL_URL: &str = "SUNRISE_PROXY_TEST_LOCAL_URL";
        let headers = [
            ("Authorization", "Bearer dummy-origin-token"),
            ("Cookie", "dummy=session"),
        ];
        if let Ok(origin) = std::env::var(LOCAL_URL) {
            let response = get(&format!("{origin}/start"), &headers, None).unwrap();
            assert_eq!(response.into_string().unwrap(), "direct again");
            let error = get(&format!("{origin}/secure"), &headers, None).unwrap_err();
            let kind = ureq::ErrorKind::ProxyConnect;
            assert!(matches!(error, GetError::Transport { kind: k, .. } if k == kind));
            assert!(error.to_string().contains(&kind.to_string()));
            assert!(error.to_string().contains("https://sunrise.invalid/asset"));

            let status = get(&format!("{origin}/status"), &headers, None).unwrap_err();
            assert_eq!(status.status(), Some(403));
            assert!(status.to_string().contains("403"));
            assert!(status.to_string().contains(&format!("{origin}/denied")));

            for message in [
                error.to_string(),
                format!("{error:?}"),
                status.to_string(),
                format!("{status:?}"),
            ] {
                for secret in ["dummy-user", "dummy-password", "dummy-signature"] {
                    assert!(!message.contains(secret), "{message}");
                }
            }
            return;
        }

        let reply = |listener: &TcpListener,
                     request_line: &str,
                     first: bool,
                     proxy_auth: Option<&str>,
                     answer: &str| {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(TIMEOUT)).unwrap();
            stream.set_write_timeout(Some(TIMEOUT)).unwrap();
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line.trim_end(), request_line);
            let mut head = String::new();
            loop {
                let n = reader.read_line(&mut head).unwrap();
                if n == 2 {
                    break;
                }
                assert_ne!(n, 0, "client disconnected before sending its headers");
            }
            assert_eq!(head.contains("dummy-origin-token"), first);
            assert_eq!(head.contains("dummy=session"), first);
            let auth = head
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("proxy-authorization"))
                .map(|(_, value)| value.trim());
            assert_eq!(auth, proxy_auth);
            stream.write_all(answer.as_bytes()).unwrap();
        };

        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let http = TcpListener::bind("127.0.0.1:0").unwrap();
        let https = TcpListener::bind("127.0.0.1:0").unwrap();
        let local_url = format!("http://localhost:{}", origin.local_addr().unwrap().port());
        let http_proxy = format!("http://{}", http.local_addr().unwrap());
        let https_proxy = format!("http://user:pass@{}", https.local_addr().unwrap());
        let to_origin = format!(
            "HTTP/1.1 302 Found\r\nLocation: {local_url}/finish\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let denied = local_url.replacen("http://", "http://dummy-user:dummy-password@", 1);
        let to_denied = format!(
            "HTTP/1.1 302 Found\r\nLocation: {denied}/denied?sig=dummy-signature\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let http = std::thread::spawn(move || {
            reply(
                &http,
                "GET http://sunrise.invalid/asset HTTP/1.1",
                false,
                None,
                &to_origin,
            )
        });
        let https = std::thread::spawn(move || {
            reply(
                &https,
                "CONNECT sunrise.invalid:443 HTTP/1.1",
                false,
                Some("basic dXNlcjpwYXNz"),
                "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
        });
        let origin = std::thread::spawn(move || {
            reply(
                &origin,
                "GET /start HTTP/1.1",
                true,
                None,
                "HTTP/1.1 302 Found\r\nLocation: http://sunrise.invalid/asset\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            reply(
                &origin,
                "GET /finish HTTP/1.1",
                false,
                None,
                "HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\ndirect again",
            );
            reply(
                &origin,
                "GET /secure HTTP/1.1",
                true,
                None,
                "HTTP/1.1 302 Found\r\nLocation: https://dummy-user:dummy-password@sunrise.invalid/asset?sig=dummy-signature\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            reply(&origin, "GET /status HTTP/1.1", true, None, &to_denied);
            reply(
                &origin,
                "GET /denied?sig=dummy-signature HTTP/1.1",
                false,
                None,
                "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        });

        // Mutate the environment only in the child, not in the parallel test runner.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "tests::redirects_select_the_proxy_for_each_destination",
            ])
            .env(LOCAL_URL, local_url)
            .env("HTTP_PROXY", http_proxy)
            .env("HTTPS_PROXY", https_proxy)
            .env("NO_PROXY", "")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        http.join().unwrap();
        https.join().unwrap();
        origin.join().unwrap();
    }

    /// The agent a second request gets is the one the first built: one connection pool, not one
    /// handshake per jar.
    #[test]
    fn an_agent_is_kept_for_the_next_request() {
        let mut agents = Agents::new();
        let mut built = Vec::new();
        for (slot, spec) in [
            ("a", None),
            ("a", Some("http://localhost:7890")),
            ("a", None),
            ("a", Some("http://localhost:7891")),
            ("a", Some("http://localhost:7890")),
            ("b", Some("http://localhost:7890")),
        ] {
            keep(
                &mut agents,
                slot,
                spec.map(|s| ("HTTP_PROXY", s.into())),
                |proxy| {
                    built.push((slot, spec));
                    let builder = ureq::AgentBuilder::new();
                    proxy.map_or_else(
                        || ureq::AgentBuilder::new().build(),
                        |p| builder.proxy(p).build(),
                    )
                },
            )
            .unwrap();
        }
        assert_eq!(
            built,
            [
                ("a", None),
                ("a", Some("http://localhost:7890")),
                ("a", Some("http://localhost:7891")),
                ("b", Some("http://localhost:7890")),
            ]
        );
    }
}
