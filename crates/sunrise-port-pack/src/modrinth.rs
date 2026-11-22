//! Modrinth metadata for the pack's files: the version of each file by
//! SHA-1, and its project (categories, licence, sides). Only metadata (the
//! jars are `fetch`'s). Answers are cached in `<work>/modrinth.json`, so a
//! rerun works offline.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};
use ureq::OrAnyStatus;

const API: &str = "https://api.modrinth.com/v2";
pub(crate) const AGENT: &str = "horizon-rust/horizon sunrise-port-pack/0.1 (docs/modpacks.md)";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    /// Version objects by file SHA-1.
    pub versions: BTreeMap<String, Value>,
    /// Project objects by project id.
    pub projects: BTreeMap<String, Value>,
}

impl Cache {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Ask Modrinth about the hashes and projects not cached yet.
    pub fn fetch(&mut self, hashes: &[String]) -> Result<(), String> {
        let missing: Vec<&String> = hashes
            .iter()
            .filter(|h| !self.versions.contains_key(*h))
            .collect();
        for chunk in missing.chunks(500) {
            let found = version_files(chunk, "sha1")?;
            for (hash, version) in found.as_object().into_iter().flatten() {
                self.versions.insert(hash.clone(), version.clone());
            }
        }
        let mut ids: Vec<String> = self
            .versions
            .values()
            .filter_map(|v| v["project_id"].as_str().map(str::to_owned))
            .filter(|id| !self.projects.contains_key(id))
            .collect();
        ids.sort();
        ids.dedup();
        for chunk in ids.chunks(100) {
            let list = serde_json::to_string(chunk).unwrap_or_default();
            let found = get(&format!("{API}/projects?ids={}", encode(&list)))?;
            for project in found.as_array().into_iter().flatten() {
                if let Some(id) = project["id"].as_str() {
                    self.projects.insert(id.to_owned(), project.clone());
                }
            }
        }
        Ok(())
    }

    pub fn project_of(&self, sha1: &str) -> Option<&Value> {
        let id = self.versions.get(sha1)?["project_id"].as_str()?;
        self.projects.get(id)
    }

    /// Required dependencies of a file's version, as project ids.
    pub fn required(&self, sha1: &str) -> Vec<String> {
        let Some(v) = self.versions.get(sha1) else {
            return Vec::new();
        };
        v["dependencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["dependency_type"] == "required")
            .filter_map(|d| d["project_id"].as_str().map(str::to_owned))
            .collect()
    }
}

fn version_files(hashes: impl Serialize, algorithm: &str) -> Result<Value, String> {
    post(
        &format!("{API}/version_files"),
        &serde_json::json!({"hashes": hashes, "algorithm": algorithm}),
    )
}

fn post(url: &str, body: &Value) -> Result<Value, String> {
    let response = sunrise_proxy::agent("port-pack-modrinth-post", url, |builder| {
        builder.redirects(0)
    })
    .map_err(|e| format!("Modrinth: {e}"))?
    .post(url)
    .set("User-Agent", AGENT)
    .set("Content-Type", "application/json")
    .send_string(&body.to_string())
    .or_any_status()
    .map_err(|e| format!("Modrinth: {}", e.kind()))?;
    read(response)
}

fn get(url: &str) -> Result<Value, String> {
    let response = sunrise_proxy::get(url, &[("User-Agent", AGENT)], None)
        .map_err(|e| format!("Modrinth: {e}"))?;
    read(response)
}

fn read(response: ureq::Response) -> Result<Value, String> {
    let status = response.status();
    if status >= 300 {
        return Err(format!("Modrinth: HTTP {status}"));
    }
    let text = response
        .into_string()
        .map_err(|e| format!("Modrinth: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("Modrinth: {e}"))
}

/// Percent-encoding for a query value.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        time::Duration,
    };

    #[test]
    #[ignore = "daily"]
    fn metadata_get_reselects_the_proxy_but_post_reports_the_redirect() {
        const ORIGIN: &str = "SUNRISE_PORT_PACK_TEST_MODRINTH_ORIGIN";
        if let Ok(base) = std::env::var(ORIGIN) {
            assert_eq!(get(&format!("{base}/get")).unwrap(), serde_json::json!([]));
            let error = post(&format!("{base}/post"), &serde_json::json!({})).unwrap_err();
            assert!(error.contains("302"), "{error}");
            return;
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let redirect = "HTTP/1.1 302 Found\r\nLocation: http://metadata.invalid/body\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            for (request_line, response) in [
                ("GET /get HTTP/1.1", redirect),
                (
                    "GET http://metadata.invalid/body HTTP/1.1",
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]",
                ),
                ("POST /post HTTP/1.1", redirect),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert_eq!(line.trim_end(), request_line);
                loop {
                    line.clear();
                    assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                    if line == "\r\n" {
                        break;
                    }
                }
                if request_line.starts_with("POST") {
                    let mut body = [0; 2];
                    reader.read_exact(&mut body).unwrap();
                    assert_eq!(&body, b"{}");
                }
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "modrinth::tests::metadata_get_reselects_the_proxy_but_post_reports_the_redirect",
            ])
            .env(ORIGIN, &base)
            .env("HTTP_PROXY", &base)
            .env("NO_PROXY", "")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        server.join().unwrap();
    }
}
