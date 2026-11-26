//! Native library build identity, shared by the game's library loader and the SDK/registry tools.
use sha2::{Digest, Sha256};

/// A native library's build identity, which the builder stamps on the library, the build record
/// keeps and the loader compares, fields separated by spaces:
/// `<build> <mod id> <version> <rustc> <source> <dependencies> <panic strategy>`.
///
/// `source` is the digest of the mod's files; a library built with crates.io dependencies adds
/// `+<graph>`, the digest of the crates.io packages it is compiled with as that build resolved
/// them (version, source and the features of each compilation unit, target and host apart), since
/// one build unifies them across every mod it compiles. `dependencies` names each native mod the library
/// links, as `<id>=<SHA-256 of that library's identity>`, sorted and joined by `,`, or `none`: a
/// library built against another build of a dependency, or of a dependency's own dependency, has
/// another identity, so the loader refuses the pair before any of their code runs.
#[derive(Debug, PartialEq, Eq)]
pub struct LibraryIdentity<'a> {
    pub build: &'a str,
    pub id: &'a str,
    pub version: &'a str,
    pub rust: &'a str,
    pub source: &'a str,
    pub dependencies: &'a str,
    pub panic: &'a str,
}

impl<'a> LibraryIdentity<'a> {
    pub fn parse(identity: &'a str) -> Option<Self> {
        let mut fields = identity.split(' ');
        let mut next = || fields.next().filter(|field| !field.is_empty());
        let parsed = Self {
            build: next()?,
            id: next()?,
            version: next()?,
            rust: next()?,
            source: next()?,
            dependencies: next()?,
            panic: next()?,
        };
        fields.next().is_none().then_some(parsed)
    }

    /// Each linked native mod's id and the SHA-256 of the identity it was built against; `None`
    /// for a malformed list.
    pub fn dependencies(&self) -> Option<Vec<(&'a str, &'a str)>> {
        if self.dependencies == "none" {
            return Some(Vec::new());
        }
        self.dependencies
            .split(',')
            .map(|dependency| {
                dependency.split_once('=').filter(|(id, digest)| {
                    !id.is_empty()
                        && digest.len() == 64
                        && digest
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
            })
            .collect()
    }
}

/// A library's identity before its panic strategy, which the library appends itself: built for
/// `build` with `rust` from the mod files `source` digests, against the native mods in `linked`
/// (each one's id and full identity).
pub fn library_identity<'a>(
    [build, id, version, rust]: [&str; 4],
    source: &str,
    linked: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> String {
    let mut dependencies: Vec<_> = linked
        .into_iter()
        .map(|(id, identity)| {
            let digest: String = Sha256::digest(identity.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            format!("{id}={digest}")
        })
        .collect();
    dependencies.sort();
    let dependencies = if dependencies.is_empty() {
        "none".to_owned()
    } else {
        dependencies.join(",")
    };
    format!("{build} {id} {version} {rust} {source} {dependencies}")
}

/// The library suffix the SDK host can load, including hosts with a `-server` suffix.
pub fn library_extension(host: &str) -> Result<&'static str, String> {
    host.split('-')
        .find_map(|part| match part {
            "darwin" => Some("dylib"),
            "linux" => Some("so"),
            "windows" => Some("dll"),
            _ => None,
        })
        .ok_or_else(|| format!("unknown native library extension for SDK host {host}"))
}
