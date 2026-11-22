//! Port-pack's allowlisted downloads.

/// GET from allowed HTTPS hosts, validating every redirect before contacting it.
pub(crate) fn get_from_hosts(url: &str, hosts: &[&str]) -> Result<Option<ureq::Response>, String> {
    match sunrise_proxy::get(
        url,
        &[("User-Agent", crate::modrinth::AGENT)],
        Some(&|url| check_url(url, hosts)),
    ) {
        Ok(response) => Ok(Some(response)),
        Err(error) if error.status() == Some(404) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn check_url(url: &url::Url, hosts: &[&str]) -> Result<(), String> {
    if url.scheme() != "https"
        || !url.host_str().is_some_and(|host| hosts.contains(&host))
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Refused download URL outside allowed HTTPS hosts".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_download_allowlist_requires_exact_https_hosts() {
        let allowed = &["cdn.modrinth.com"];
        assert!(
            check_url(
                &url::Url::parse("https://cdn.modrinth.com/a").unwrap(),
                allowed
            )
            .is_ok()
        );
        for url in [
            "http://cdn.modrinth.com/a",
            "https://cdn.modrinth.com.evil/a",
            "https://user@cdn.modrinth.com/a",
            "https://cdn.modrinth.com:444/a",
        ] {
            assert!(check_url(&url::Url::parse(url).unwrap(), allowed).is_err());
        }
    }
}
