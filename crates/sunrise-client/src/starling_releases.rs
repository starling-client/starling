//! Where Starling's releases come out, and what of a release is Starling's: for the update prompt
//! (game/update_offer.rs) and the native mod builds (game/native_builds.rs), on Android.
//!
//! Starling and Horizon Android are one project now, in the organisation's repository
//! (starling-client/starling); Starling's own (JuanLuci/starling) is where its releases came out
//! before, and still do until the merged one. Both are asked, the organisation's first, so an
//! install follows the releases wherever they come out. The organisation's repository also holds
//! Horizon Android's releases (its APKs are named after it) and preview builds of Starling
//! (`starling-preview-...`): only an APK named `starling-<version>-...` is an update.

/// The repositories, as GitHub's API names them, the organisation's first.
pub const REPOSITORIES: [&str; 2] = ["starling-client/starling", "JuanLuci/starling"];

/// A repository's latest release, as GitHub's API serves it.
pub fn latest_url(repository: &str) -> String {
    format!("https://api.github.com/repos/{repository}/releases/latest")
}

/// A repository's release of native mod builds (native-mods.yml publishes to it).
pub fn native_builds_url(repository: &str) -> String {
    format!("https://api.github.com/repos/{repository}/releases/tags/native-builds")
}

/// Whether version `latest` (a tag: `v0.1.2`) comes after `current` (`0.1.1`), number by number.
pub fn newer(latest: &str, current: &str) -> bool {
    let numbers = |v: &str| -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['.', '-', '+'])
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    };
    numbers(latest) > numbers(current)
}

/// A Starling release's APK.
#[derive(Clone, Debug, PartialEq)]
pub struct Update {
    /// Without its `v`: `0.8.0`.
    pub version: String,
    /// The APK's file name, its download and its size.
    pub name: String,
    pub url: String,
    pub size: u64,
    /// The SHA-256 GitHub gives for it (hex), checked after the download.
    pub sha256: Option<String>,
}

/// The Android ABI of this process (not every ABI the device can run).
pub const fn process_abi() -> &'static str {
    if cfg!(target_arch = "arm") {
        "armeabi-v7a"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "arm64-v8a"
    }
}

/// Whether a native build's metadata targets this process. Metadata published
/// before the ABI field existed is ARM64, the service's original sole target.
pub fn native_build_matches(about: &serde_json::Value, abi: &str) -> bool {
    match about.get("abi") {
        None => abi == "arm64-v8a",
        Some(value) => value.as_str() == Some(abi),
    }
}

/// The release `latest` as an update when it has a newer APK for this process.
pub fn update_in(latest: &serde_json::Value, current: &str) -> Option<Update> {
    update_for_abi(latest, current, process_abi())
}

fn update_for_abi(latest: &serde_json::Value, current: &str, abi: &str) -> Option<Update> {
    let tag = latest["tag_name"].as_str()?;
    if !newer(tag, current) {
        return None;
    }
    let version = tag.trim_start_matches('v');
    let name = format!("starling-{version}-{abi}.apk");
    let apk = latest["assets"]
        .as_array()?
        .iter()
        .find(|a| a["name"].as_str() == Some(name.as_str()))?;
    Some(Update {
        version: version.to_owned(),
        name: apk["name"].as_str()?.to_owned(),
        url: apk["browser_download_url"]
            .as_str()
            .filter(|u| u.starts_with("https://"))?
            .to_owned(),
        size: apk["size"].as_u64().unwrap_or(0),
        sha256: apk["digest"]
            .as_str()
            .and_then(|d| d.strip_prefix("sha256:"))
            .map(str::to_owned),
    })
}

/// The newest of the updates the repositories offer; the first of two of one version.
pub fn newest(updates: impl IntoIterator<Item = Update>) -> Option<Update> {
    updates
        .into_iter()
        .fold(None, |best: Option<Update>, u| match best {
            Some(b) if !newer(&u.version, &b.version) => Some(b),
            _ => Some(u),
        })
}

/// Where Starling's own build service is: a file on the organisation's protected branch, so the
/// service can move without an update.
pub const BUILD_SERVER_URL: &str = "https://raw.githubusercontent.com/starling-client/starling/starling/packaging/android/build-server.txt";

/// `text` as a build server's address, when it is one: HTTPS to a host, maybe with a port, and
/// nothing else (no account, path or query to hide where it really goes).
pub fn build_server_address(text: &str) -> Option<String> {
    let address = text.trim().trim_end_matches('/').to_ascii_lowercase();
    let rest = address.strip_prefix("https://")?;
    let (host, port) = rest.split_once(':').unwrap_or((rest, ""));
    let host_is_plain = !host.is_empty()
        && host.len() <= 253
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-']);
    let port_is_plain = rest.len() == host.len() || port.parse::<u16>().is_ok_and(|port| port != 0);
    (host_is_plain && port_is_plain).then_some(address)
}

/// A checked build server address as a player reads it: without `https://`.
pub fn build_server_host(address: &str) -> &str {
    address.strip_prefix("https://").unwrap_or(address)
}

/// A build of version's game with native mods, in a native-builds release.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeBuild {
    /// `v0.1.4__wrench`: its assets' names, and its folder on the phone.
    pub name: String,
    pub mods: Vec<String>,
    /// Its JSON's and its library's downloads.
    pub json: String,
    pub library: String,
}

/// A name of the release's assets: lowercase letters, digits, `.`, `_` and `-`.
fn plain(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// The builds made for `version` in `release` (a native-builds release as the API serves it).
pub fn native_builds_in(release: &serde_json::Value, version: &str) -> Vec<NativeBuild> {
    let assets = release["assets"].as_array().cloned().unwrap_or_default();
    let url = |name: &str| {
        assets
            .iter()
            .find(|a| a["name"].as_str() == Some(name))
            .and_then(|a| a["browser_download_url"].as_str())
            .filter(|u| u.starts_with("https://"))
            .map(str::to_owned)
    };
    let prefix = format!("v{version}__");
    assets
        .iter()
        .filter_map(|a| a["name"].as_str())
        .filter_map(|n| n.strip_suffix(".json"))
        .filter(|n| n.starts_with(&prefix) && plain(n))
        .filter_map(|name| {
            Some(NativeBuild {
                name: name.to_owned(),
                mods: name[prefix.len()..]
                    .split("__")
                    .map(str::to_owned)
                    .collect(),
                json: url(&format!("{name}.json"))?,
                library: url(&format!("{name}.so"))?,
            })
        })
        .collect()
}

/// The repositories' builds as one list: a set of mods once (the first repository's), the fewest
/// mods first.
pub fn merge_native_builds(lists: impl IntoIterator<Item = Vec<NativeBuild>>) -> Vec<NativeBuild> {
    let mut builds: Vec<NativeBuild> = Vec::new();
    for build in lists.into_iter().flatten() {
        if !builds.iter().any(|b| b.name == build.name) {
            builds.push(build);
        }
    }
    builds.sort_by(|a, b| (a.mods.len(), &a.name).cmp(&(b.mods.len(), &b.name)));
    builds
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_build_metadata_must_match_the_process_before_downloading() {
        let abis = ["arm64-v8a", "armeabi-v7a", "x86", "x86_64"];
        for target in abis {
            assert_eq!(
                native_build_matches(&json!({}), target),
                target == "arm64-v8a"
            );
            assert!(!native_build_matches(&json!({"abi":null}), target));
            assert!(!native_build_matches(&json!({"abi":"unknown"}), target));
            for build in abis {
                assert_eq!(
                    native_build_matches(&json!({"abi":build}), target),
                    build == target
                );
            }
        }
    }

    fn asset(name: &str) -> serde_json::Value {
        json!({
            "name": name,
            "browser_download_url": format!("https://github.com/x/releases/download/t/{name}"),
            "size": 10,
            "digest": "sha256:ab",
        })
    }

    #[test]
    fn updates_select_the_process_abi_even_when_another_apk_is_listed_first() {
        let all = json!({"tag_name":"v0.3.0", "assets":[
            asset("starling-0.3.0-arm64-v8a.apk"), asset("starling-0.3.0-armeabi-v7a.apk"),
            asset("starling-0.3.0-x86.apk"), asset("starling-0.3.0-x86_64.apk")
        ]});
        for abi in ["arm64-v8a", "armeabi-v7a", "x86", "x86_64"] {
            assert_eq!(
                update_for_abi(&all, "0.2.3", abi).unwrap().name,
                format!("starling-0.3.0-{abi}.apk")
            );
        }
        let release = json!({"tag_name":"v0.3.0", "assets":[
            asset("starling-0.3.0-arm64-v8a.apk"), asset("starling-0.3.0-armeabi-v7a.apk")
        ]});
        assert_eq!(
            update_for_abi(&release, "0.2.3", "armeabi-v7a")
                .unwrap()
                .name,
            "starling-0.3.0-armeabi-v7a.apk"
        );
        assert_eq!(
            update_for_abi(&release, "0.2.3", "arm64-v8a").unwrap().name,
            "starling-0.3.0-arm64-v8a.apk"
        );
        let arm64_only =
            json!({"tag_name":"v0.3.0", "assets":[asset("starling-0.3.0-arm64-v8a.apk")]});
        assert!(update_for_abi(&arm64_only, "0.2.3", "armeabi-v7a").is_none());
    }

    #[test]
    fn a_build_server_is_https_to_a_host_and_nothing_else() {
        for (text, address) in [
            ("https://builds.example.com", "https://builds.example.com"),
            (
                " HTTPS://Builds.Example.com/\n",
                "https://builds.example.com",
            ),
            (
                "https://203-0-113-7.sslip.io:8443",
                "https://203-0-113-7.sslip.io:8443",
            ),
        ] {
            assert_eq!(build_server_address(text).as_deref(), Some(address));
        }
        for text in [
            "",
            "https://",
            "builds.example.com",
            "http://builds.example.com",
            "https://user:secret@builds.example.com",
            "https://builds.example.com/path",
            "https://builds.example.com?to=elsewhere",
            "https://builds.example.com#elsewhere",
            "https://builds example.com",
            "https://.example.com",
            "https://example.com-",
            "https://example.com:",
            "https://example.com:0",
            "https://example.com:99999",
            "https://example.com:80:80",
        ] {
            assert_eq!(build_server_address(text), None, "{text}");
        }
    }

    #[test]
    fn the_published_build_server_is_a_good_address() {
        let published = include_str!("../../../packaging/android/build-server.txt");
        assert!(build_server_address(published).is_some());
    }

    #[test]
    fn a_build_server_reads_without_its_scheme() {
        assert_eq!(
            build_server_host("https://example.com:8443"),
            "example.com:8443"
        );
    }

    #[test]
    fn versions_compare_number_by_number() {
        assert!(newer("v0.8.0", "0.2.2"));
        assert!(newer("v0.2.10", "0.2.9"));
        assert!(!newer("v0.2.2", "0.2.2"));
        assert!(!newer("v0.1.9", "0.2.0"));
    }

    #[test]
    fn only_starlings_own_apk_is_an_update() {
        // Horizon Android's last release, the organisation's latest today: not Starling's.
        let sunrise =
            json!({"tag_name": "v0.7.8", "assets": [asset("Sunrise-Android-0.7.8-arm64-v8a.apk")]});
        assert_eq!(update_for_abi(&sunrise, "0.2.2", "arm64-v8a"), None);
        // A preview build is another app, signed with another key.
        let preview = json!({"tag_name": "v0.8.0", "assets": [asset("starling-preview-0.8.0-arm64-v8a.apk")]});
        assert_eq!(update_for_abi(&preview, "0.2.2", "arm64-v8a"), None);
        let release = json!({"tag_name": "v0.8.0", "assets": [
            asset("starling-preview-0.8.0-arm64-v8a.apk"),
            asset("StarlingVisuals.zip"),
            asset("starling-0.8.0-arm64-v8a.apk"),
        ]});
        let update = update_for_abi(&release, "0.2.2", "arm64-v8a").unwrap();
        assert_eq!(update.version, "0.8.0");
        assert_eq!(update.name, "starling-0.8.0-arm64-v8a.apk");
        assert_eq!(update.sha256.as_deref(), Some("ab"));
        assert_eq!(
            update_for_abi(&release, "0.8.0", "arm64-v8a"),
            None,
            "not newer"
        );
    }

    #[test]
    fn the_newest_repository_wins_and_a_tie_keeps_the_first() {
        let update = |version: &str, url: &str| Update {
            version: version.into(),
            name: format!("starling-{version}-arm64-v8a.apk"),
            url: url.into(),
            size: 1,
            sha256: None,
        };
        let org = update("0.8.0", "org");
        let own = update("0.2.3", "own");
        assert_eq!(newest([own.clone(), org.clone()]), Some(org.clone()));
        assert_eq!(
            newest([org.clone(), update("0.8.0", "own")]).unwrap().url,
            "org"
        );
        assert_eq!(newest([]), None);
    }

    #[test]
    fn native_builds_merge_by_name_the_first_repository_first() {
        let release = |base: &str| {
            json!({"assets": [
                asset("v0.2.2__wrench.json"), asset("v0.2.2__wrench.so"),
                asset("v0.2.2__wrench__carry.json"), asset("v0.2.2__wrench__carry.so"),
                asset("v0.2.1__old.json"), asset("v0.2.1__old.so"),
                asset(&format!("v0.2.2__{base}.json")),
            ]})
        };
        let org = native_builds_in(&release("lonely"), "0.2.2");
        assert_eq!(org.len(), 2, "a build needs its library, and this version");
        let own = native_builds_in(&release("other"), "0.2.2");
        let merged = merge_native_builds([org, own]);
        assert_eq!(
            merged.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
            ["v0.2.2__wrench", "v0.2.2__wrench__carry"]
        );
        assert_eq!(merged[1].mods, ["wrench", "carry"]);
    }
}
