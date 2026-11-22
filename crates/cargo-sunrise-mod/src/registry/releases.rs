//! Resolve published stable GitHub releases to the exact commits their tags name.
use crate::internals::atomic_write;
use serde::Deserialize;
use std::{collections::BTreeSet, env, io::Read, path::Path, time::Duration};
use sunrise_addresses::{GAME_GIT_API, SDK_RELEASE_LIST};
use sunrise_mod_package::{
    index::{registry::ReleaseInput, stable_tag},
    manifest::revision,
};

const PAGE_SIZE: usize = 100; // SDK_RELEASE_LIST asks GitHub for this many rows.
const MAX_PAGES: usize = 10_000;
const MAX_TAG_DEPTH: usize = 128;
const MAX_RESPONSE: u64 = 32 << 20;

struct Response {
    body: Vec<u8>,
    links: Vec<String>,
}

pub(super) fn fetch(out: &Path) -> Result<(), String> {
    let mut auth = None;
    for name in ["GH_TOKEN", "GITHUB_TOKEN"] {
        match env::var(name) {
            Ok(token) if token.is_empty() => continue,
            Ok(token) if token.bytes().all(|b| b.is_ascii_graphic()) => {
                auth = Some(format!("Bearer {token}"));
                break;
            }
            Err(env::VarError::NotPresent) => continue,
            _ => return Err(format!("{name} is not a valid GitHub token")),
        }
    }
    // Request only the fixed Sunrise repository. Do not forward credentials to redirects or
    // to URLs supplied by pagination headers or Git objects.
    write_list(out, |url| {
        let agent = sunrise_proxy::agent("cargo-sunrise-mod-registry", url, |builder| {
            builder.redirects(0).timeout(Duration::from_secs(30))
        })
        .map_err(|e| format!("GitHub request failed: {e}"))?;
        let mut request = agent
            .get(url)
            .set("Accept", "application/vnd.github+json")
            .set("User-Agent", "cargo-sunrise-mod");
        if let Some(auth) = &auth {
            request = request.set("Authorization", auth);
        }
        let response = request.call().map_err(|error| match error {
            ureq::Error::Status(status, _) => format!("GitHub returned HTTP {status}"),
            ureq::Error::Transport(error) => format!("GitHub request failed: {:?}", error.kind()),
        })?;
        if response.status() != 200 {
            return Err(format!(
                "GitHub returned HTTP {}, expected 200",
                response.status()
            ));
        }
        let links = response
            .all("Link")
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut body = Vec::new();
        response
            .into_reader()
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut body)
            .map_err(|e| format!("could not read the whole GitHub response: {e}"))?;
        if body.len() as u64 > MAX_RESPONSE {
            return Err("GitHub response exceeds the release discovery size limit".into());
        }
        Ok(Response { body, links })
    })
}

fn write_list(
    out: &Path,
    fetch: impl FnMut(&str) -> Result<Response, String>,
) -> Result<(), String> {
    let releases = discover(fetch)?;
    let mut text = serde_json::to_string(&releases).map_err(|e| e.to_string())?;
    text.push('\n');
    atomic_write(out, text)?;
    println!("Wrote {}", out.display());
    Ok(())
}

#[derive(Deserialize)]
struct Release {
    id: u64,
    tag_name: String,
    draft: bool,
    prerelease: bool,
    // Null means unpublished; unlike Option, this requires the field to be present.
    published_at: PublishedAt,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PublishedAt {
    At(String),
    Unpublished(()),
}

impl PublishedAt {
    fn is_published(&self) -> Result<bool, &'static str> {
        match self {
            Self::At(timestamp) => chrono::DateTime::parse_from_rfc3339(timestamp)
                .map(|_| true)
                .map_err(|_| "published_at must be an RFC 3339 timestamp or null"),
            Self::Unpublished(()) => Ok(false),
        }
    }
}

#[derive(Default)]
struct PageLinks {
    next: Option<usize>,
    last: Option<usize>,
    first: Option<usize>,
    previous: Option<usize>,
}

impl PageLinks {
    fn read(headers: &[String]) -> Result<Self, String> {
        let mut pages = Self::default();
        let invalid = || "malformed GitHub releases pagination header".to_owned();
        let (endpoint, _) = SDK_RELEASE_LIST
            .split_once('?')
            .expect("release list query");
        let (origin, _) = endpoint
            .split_once("/repos/")
            .expect("release list repository");
        for link in headers.iter().flat_map(|header| header.split(',')) {
            let (url, rel) = link
                .trim()
                .strip_prefix('<')
                .and_then(|link| link.split_once('>'))
                .and_then(|(url, attrs)| {
                    Some((
                        url,
                        attrs
                            .trim()
                            .strip_prefix(';')?
                            .trim()
                            .strip_prefix("rel=\"")?
                            .strip_suffix('"')?,
                    ))
                })
                .ok_or_else(invalid)?;
            let slot = match rel {
                "next" => &mut pages.next,
                "last" => &mut pages.last,
                "first" => &mut pages.first,
                "prev" => &mut pages.previous,
                _ => return Err(invalid()),
            };
            let (path, query) = url.split_once('?').ok_or_else(invalid)?;
            // GitHub can spell the repository by numeric ID; the URL itself is never fetched.
            if path != endpoint
                && !path
                    .strip_prefix(origin)
                    .and_then(|path| path.strip_prefix("/repositories/"))
                    .and_then(|path| path.strip_suffix("/releases"))
                    .is_some_and(|id| {
                        !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
                    })
            {
                return Err(invalid());
            }
            let mut page = None;
            let mut size = None;
            for param in query.split('&') {
                let (key, value) = param.split_once('=').ok_or_else(invalid)?;
                let value = value
                    .parse::<usize>()
                    .ok()
                    .filter(|&value| value > 0)
                    .ok_or_else(invalid)?;
                match key {
                    "page" if page.is_none() => page = Some(value),
                    "per_page" if size.is_none() && value == PAGE_SIZE => size = Some(value),
                    _ => return Err(invalid()),
                }
            }
            if slot.replace(page.ok_or_else(invalid)?).is_some() {
                return Err(invalid());
            }
        }
        Ok(pages)
    }
}

fn discover(
    mut fetch: impl FnMut(&str) -> Result<Response, String>,
) -> Result<Vec<ReleaseInput>, String> {
    let mut tags = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut page = 1;
    let mut last = None;
    let mut probing_end = false;
    loop {
        let at = |e| format!("Sunrise releases page {page}: {e}");
        let response = fetch(&format!("{SDK_RELEASE_LIST}&page={page}")).map_err(at)?;
        let links = PageLinks::read(&response.links).map_err(at)?;
        let rows: Vec<Release> =
            serde_json::from_slice(&response.body).map_err(|e| at(e.to_string()))?;
        if links.first.is_some_and(|first| first != 1)
            || links.previous.is_some_and(|previous| previous != page - 1)
        {
            return Err(at(
                "pagination did not point back to the expected pages".into()
            ));
        }
        if probing_end {
            if !rows.is_empty()
                || links.next.is_some()
                || links
                    .last
                    .is_some_and(|end| !(page - 1..=page).contains(&end))
            {
                return Err(at(
                    "expected an empty page with no continuation after the full page".into(),
                ));
            }
            break;
        }
        if let Some(end) = links.last {
            if end < page || last.is_some_and(|old| old != end) {
                return Err(at("the last page changed during listing".into()));
            }
            last = Some(end);
        }
        let full = rows.len() == PAGE_SIZE;
        if rows.len() > PAGE_SIZE || (page > 1 && rows.is_empty()) {
            return Err(at("incomplete or inconsistent page".into()));
        }
        if let Some(next) = links.next {
            if !full || next != page + 1 || last.is_some_and(|end| next > end) {
                return Err(at(
                    "inconsistent page with an advertised continuation".into()
                ));
            }
        } else if last.is_some_and(|end| page < end) {
            return Err(at(
                "missing continuation for a page not known to be last".into()
            ));
        }
        for release in rows {
            let published = release
                .published_at
                .is_published()
                .map_err(|e| at(format!("release {:?}: {e}", release.tag_name)))?;
            if !ids.insert(release.id) {
                return Err(at(format!(
                    "release {} appeared twice during listing",
                    release.id
                )));
            }
            if !release.draft
                && !release.prerelease
                && published
                && stable_tag(&release.tag_name).is_some()
            {
                tags.insert(release.tag_name);
            }
        }
        if links.next.is_none() {
            if !full || last == Some(page) {
                break;
            }
            // A full page with no advertised end could be the final page. Probe once at
            // the fixed endpoint; do not accept rows from an unadvertised continuation.
            probing_end = true;
        }
        page += 1;
        if page > MAX_PAGES {
            return Err("Sunrise release listing exceeded its page limit".into());
        }
    }
    tags.into_iter()
        .map(|tag| {
            let commit =
                peel(&tag, &mut fetch).map_err(|e| format!("Sunrise release {tag}: {e}"))?;
            Ok(ReleaseInput { tag, commit })
        })
        .collect()
}

#[derive(Deserialize)]
struct GitObject {
    #[serde(rename = "type")]
    kind: String,
    sha: String,
}

fn peel(
    tag: &str,
    fetch: &mut impl FnMut(&str) -> Result<Response, String>,
) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Ref {
        object: GitObject,
    }
    let mut url = format!("{GAME_GIT_API}/ref/tags/{tag}");
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_TAG_DEPTH {
        let response = fetch(&url)?;
        let Ref { object } = serde_json::from_slice(&response.body).map_err(|e| e.to_string())?;
        if !revision(&object.sha) {
            return Err("GitHub did not return a full lowercase Git object ID".into());
        }
        match object.kind.as_str() {
            "commit" => return Ok(object.sha),
            "tag" => {
                url = format!("{GAME_GIT_API}/tags/{}", object.sha);
                if !seen.insert(object.sha) {
                    return Err("annotated tag cycle".into());
                }
            }
            _ => return Err("tag does not point to a commit".into()),
        }
    }
    Err("annotated tag chain is too deep".into())
}

#[cfg(test)]
mod tests;
