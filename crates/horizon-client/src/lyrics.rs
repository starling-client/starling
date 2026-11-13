//! The lrclib.net lookup behind capability `lyrics` (docs/mods.md): one
//! HTTPS GET per new track; the title, artist, album and length are the
//! only things sent. Kept free of the mod API so it can be tested on its
//! own.

/// The lyrics of one track, as a lookup found them.
#[derive(Clone, Debug, PartialEq)]
pub struct Lyrics {
    /// True for a track with no sung words.
    pub instrumental: bool,
    /// (start, text), start in milliseconds from the track's start.
    pub lines: Vec<(u64, String)>,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(std::time::Duration::from_secs(8))
        .timeout_read(std::time::Duration::from_secs(15))
        .build()
}

fn percent(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// One line of an LRC file: `[mm:ss.xx] text` (also `mm:ss` and `.xxx`).
/// A `[offset:±ms]` tag, anywhere in the file, shifts every line of it
/// (positive shows the lines earlier, as players read it).
pub fn parse_lrc(text: &str) -> Vec<(u64, String)> {
    let mut offset_ms: i64 = 0;
    for line in text.lines() {
        for inner in tags(line) {
            if let Some(value) = inner.trim().strip_prefix("offset:")
                && let Ok(value) = value.trim().parse::<i64>()
            {
                offset_ms = value;
            }
        }
    }
    let mut out = Vec::new();
    for line in text.lines() {
        let mut times = Vec::new();
        let mut rest = line.trim();
        while let Some(body) = rest.strip_prefix('[') {
            let Some(close) = body.find(']') else { break };
            let inner = body[..close].trim();
            rest = body[close + 1..].trim_start();
            if let Some(t) = parse_tag(inner) {
                times.push(t);
            }
        }
        let text = rest.trim();
        if times.is_empty() || text.is_empty() {
            continue;
        }
        for t in &times {
            let at = (*t as i64 - offset_ms).max(0) as u64;
            out.push((at, text.to_string()));
        }
    }
    out.sort_by_key(|(at, _)| *at);
    out
}

/// The `[...]` groups at the start of a line, in order.
fn tags(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = line.trim();
    while let Some(body) = rest.strip_prefix('[') {
        let Some(close) = body.find(']') else { break };
        out.push(body[..close].trim());
        rest = body[close + 1..].trim_start();
    }
    out
}

fn parse_tag(inner: &str) -> Option<u64> {
    let (mm, rest) = inner.split_once(':')?;
    let mm: u64 = mm.trim().parse().ok()?;
    let (ss, frac) = match rest.split_once('.') {
        Some((ss, frac)) => (ss, Some(frac.trim())),
        None => (rest, None),
    };
    let ss: u64 = ss.trim().parse().ok()?;
    if ss >= 60 {
        return None;
    }
    let ms = match frac {
        None => 0,
        Some(frac) => {
            let mut digits: String = frac.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.is_empty() {
                return None;
            }
            if digits.len() > 3 {
                digits.truncate(3);
            }
            while digits.len() < 3 {
                digits.push('0');
            }
            digits.parse().ok()?
        }
    };
    Some(mm * 60_000 + ss * 1_000 + ms)
}

/// The lrclib.net answers for one track; the `get` endpoint first, then a
/// `search` filtered to a timed match.
pub fn fetch_lrclib(
    title: &str,
    artist: &str,
    album: &str,
    duration_seconds: f64,
) -> Result<Lyrics, String> {
    let duration = if duration_seconds.is_finite() {
        duration_seconds.round().clamp(0.0, 86_400.0) as u64
    } else {
        0
    };
    let query = format!(
        "artist_name={}&track_name={}&album_name={}&duration={duration}",
        percent(artist),
        percent(title),
        percent(album)
    );
    let get = agent()
        .get(&format!("https://lrclib.net/api/get?{query}"))
        .call();
    match get {
        Ok(response) => {
            let body = response.into_string().map_err(|e| format!("lrclib: {e}"))?;
            let parsed: serde_json::Value = serde_json::from_str(&body)
                .map_err(|_| "lrclib answered with invalid JSON".to_string())?;
            // A get without synced lyrics is not the end: it returned a
            // track with plain lyrics only, and search may carry syncs.
            if let Some(found) = from_lrclib_track(&parsed) {
                return Ok(found);
            }
        }
        Err(ureq::Error::Status(404, _)) => {}
        Err(ureq::Error::Status(code, _)) => {
            return Err(format!("lrclib answered with HTTP {code}"));
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(format!("lrclib.net is unreachable ({})", t.kind()));
        }
    }
    let search = agent()
        .get(&format!(
            "https://lrclib.net/api/search?artist_name={}&track_name={}",
            percent(artist),
            percent(title)
        ))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(code, _) => format!("lrclib answered with HTTP {code}"),
            ureq::Error::Transport(t) => format!("lrclib.net is unreachable ({})", t.kind()),
        })?;
    let body = search.into_string().map_err(|e| format!("lrclib: {e}"))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|_| "lrclib answered with invalid JSON".to_string())?;
    let tracks = parsed.as_array().ok_or("lrclib answered with no results")?;
    // The closest duration with synced lyrics, else any with synced lyrics.
    let mut best: Option<(u64, &serde_json::Value)> = None;
    for track in tracks {
        let Some(synced) = track.get("syncedLyrics").and_then(|v| v.as_str()) else {
            continue;
        };
        if synced.trim().is_empty() {
            continue;
        }
        let length = track
            .get("duration")
            .and_then(|v| v.as_f64())
            .map(|d| d.round() as u64)
            .unwrap_or(0);
        let distance = length.abs_diff(duration);
        if best.is_none_or(|(d, _)| distance < d) {
            best = Some((distance, track));
        }
    }
    match best {
        Some((_, track)) => from_lrclib_track(track)
            .ok_or_else(|| "no synced lyrics found for this track".to_string()),
        None => Err("no synced lyrics found for this track".to_string()),
    }
}

fn from_lrclib_track(track: &serde_json::Value) -> Option<Lyrics> {
    let instrumental = track
        .get("instrumental")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if instrumental {
        return Some(Lyrics {
            instrumental: true,
            lines: Vec::new(),
        });
    }
    let synced = track
        .get("syncedLyrics")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let lines = parse_lrc(synced);
    if lines.is_empty() {
        return None;
    }
    Some(Lyrics {
        instrumental: false,
        lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_lines_with_tag_forms() {
        let lrc = "[ti: Title]\n[ar: Artist]\n[offset:+1000]\n[00:12.34] first line\n[01:02] second\n[02:03.999] third\n[00:12.50][03:04.5] two times\n\n[04:05] no alpha\n";
        assert_eq!(
            parse_lrc(lrc),
            vec![
                (11_340, "first line".to_string()),
                (11_500, "two times".to_string()),
                (61_000, "second".to_string()),
                (122_999, "third".to_string()),
                (183_500, "two times".to_string()),
                (244_000, "no alpha".to_string()),
            ]
        );
    }

    #[test]
    fn lrc_without_tags_has_no_lines() {
        assert!(parse_lrc("just some words").is_empty());
        assert!(parse_lrc("[05:99] bad seconds").is_empty());
    }
}
