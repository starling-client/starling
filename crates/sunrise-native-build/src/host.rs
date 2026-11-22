//! What the program running a build provides: its language, the build's folder, the network, cache stamps.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// A body to read, or `None` when the server answered 404.
pub type Body = Option<Box<dyn Read + Send>>;

/// The program's side of a build. Each is a plain function, so a host reads its own state.
#[derive(Clone, Copy)]
pub struct Host {
    /// `en` in the program's language, as the launcher's `i18n::tr` does: English is the key.
    pub tr: for<'a> fn(&'a str) -> &'a str,
    /// The build's own folder: tools, sources, kept compiles and `build.log`.
    pub root: fn() -> PathBuf,
    /// A GET of `url` through the program's network settings.
    pub get: fn(&str) -> Result<Body, String>,
    /// Records that the cache folder or file at the path was used, for the program's cleanup.
    pub mark_used: fn(&Path) -> Result<(), String>,
}

static HOST: OnceLock<Host> = OnceLock::new();

/// What a build uses before a program installs its own: English, a scratch folder, no network.
const DEFAULT: Host = Host {
    tr: |en| en,
    root: || std::env::temp_dir().join(format!("sunrise-native-build-{}", std::process::id())),
    get: |url| Err(format!("{url}: this program has no network")),
    mark_used: |_| Ok(()),
};

/// Makes `host` the process's; the first one installed stays.
pub fn install(host: Host) {
    let _ = HOST.set(host);
}

fn host() -> &'static Host {
    HOST.get().unwrap_or(&DEFAULT)
}

/// `en` in the host's language.
pub fn tr(en: &str) -> &str {
    (host().tr)(en)
}

/// `en` in the host's language with its `{name}` holes filled.
pub fn trf(en: &str, args: &[(&str, &str)]) -> String {
    let mut out = tr(en).to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// The build's own folder.
pub fn root() -> PathBuf {
    (host().root)()
}

/// The body at `url`, or `None` for a 404.
pub fn get_unless_missing(url: &str) -> Result<Body, String> {
    (host().get)(url)
}

/// The body at `url`; a 404 is an error.
pub fn get(url: &str) -> Result<Box<dyn Read + Send>, String> {
    get_unless_missing(url)?.ok_or_else(|| format!("{url} answered 404"))
}

/// Stamps the cache folder or file `path` as used.
pub fn mark_used(path: &Path) -> Result<(), String> {
    (host().mark_used)(path)
}
