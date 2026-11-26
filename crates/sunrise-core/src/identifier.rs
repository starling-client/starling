//! Validated resource identifiers, with Java hashing and path-first ordering.
use crate::java_hash_map::JavaKey;
use std::{alloc::Layout, cmp::Ordering, fmt, sync::Arc};
#[derive(Debug, PartialEq, Eq)]
struct Data {
    text: Box<str>,
    colon: usize,
    hash: i32,
}

/// Layout of the shared value, excluding the Arc header and string buffers.
#[doc(hidden)]
pub const IDENTIFIER_DATA_LAYOUT: Layout = Layout::new::<Data>();

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identifier(Arc<Data>);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentifierError {
    Namespace,
    Path,
}
impl fmt::Display for IdentifierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Namespace => "invalid identifier namespace",
            Self::Path => "invalid identifier path",
        })
    }
}
impl std::error::Error for IdentifierError {}
fn valid(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c)
}
fn hash(s: &str) -> i32 {
    s.bytes()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
}
fn validate_parts(namespace: &str, path: &str) -> Result<(), IdentifierError> {
    if namespace == ".." || !namespace.bytes().all(valid) {
        return Err(IdentifierError::Namespace);
    }
    if !path.bytes().all(|c| valid(c) || c == b'/') {
        return Err(IdentifierError::Path);
    }
    Ok(())
}

const DEFAULT_NAMESPACE: &str = "minecraft";

fn split_identifier(text: &str) -> (Option<&str>, &str) {
    match text.split_once(':') {
        Some(("", path)) => (None, path),
        Some((namespace, path)) => (Some(namespace), path),
        None => (None, text),
    }
}

fn validated_parts(text: &str) -> Result<(Option<&str>, &str), IdentifierError> {
    let (namespace, path) = split_identifier(text);
    validate_parts(namespace.unwrap_or(DEFAULT_NAMESPACE), path)?;
    Ok((namespace, path))
}

/// Validate a borrowed identifier without allocating its name.
pub fn validate(text: &str) -> Result<(), IdentifierError> {
    validated_parts(text).map(|_| ())
}

/// Validate identifier text and supply the default namespace when omitted.
/// Already qualified text keeps its original allocation.
pub fn canonicalize(text: String) -> Result<String, IdentifierError> {
    let (namespace, path) = validated_parts(&text)?;
    if namespace.is_some() {
        Ok(text)
    } else {
        Ok(format!("{DEFAULT_NAMESPACE}:{path}"))
    }
}

impl Identifier {
    pub fn from_parts(namespace: &str, path: &str) -> Result<Self, IdentifierError> {
        validate_parts(namespace, path)?;
        Ok(Self(Arc::new(Data {
            text: format!("{namespace}:{path}").into_boxed_str(),
            colon: namespace.len(),
            hash: hash(namespace).wrapping_mul(31).wrapping_add(hash(path)),
        })))
    }
    /// A fully qualified identifier with nonempty namespace and path.
    pub fn parse_namespaced(text: &str) -> Option<Self> {
        let (namespace, path) = text.split_once(':')?;
        if namespace.is_empty() || path.is_empty() {
            return None;
        }
        Self::from_parts(namespace, path).ok()
    }
    pub fn parse(text: &str) -> Result<Self, IdentifierError> {
        let (namespace, path) = split_identifier(text);
        Self::from_parts(namespace.unwrap_or(DEFAULT_NAMESPACE), path)
    }
    /// Borrow this identifier's fully qualified `namespace:path` spelling.
    pub fn as_str(&self) -> &str {
        &self.0.text
    }
    pub fn namespace(&self) -> &str {
        &self.as_str()[..self.0.colon]
    }
    pub fn path(&self) -> &str {
        &self.as_str()[self.0.colon + 1..]
    }
    pub fn hash_code(&self) -> i32 {
        self.0.hash
    }
}
impl Ord for Identifier {
    fn cmp(&self, other: &Self) -> Ordering {
        self.path()
            .cmp(other.path())
            .then_with(|| self.namespace().cmp(other.namespace()))
    }
}
impl PartialOrd for Identifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl JavaKey for Identifier {
    fn java_hash(&self) -> i32 {
        self.hash_code()
    }
    fn comparable_compare(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
    fn tie_break(&self, other: &Self) -> std::cmp::Ordering {
        self.cmp(other)
    }
}

impl From<&Identifier> for String {
    fn from(id: &Identifier) -> Self {
        id.to_string()
    }
}

impl TryFrom<String> for Identifier {
    type Error = IdentifierError;
    fn try_from(text: String) -> Result<Self, IdentifierError> {
        Self::parse(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Identifier.isValidNamespace` rejects the namespace `..` and nothing else that has valid
    /// characters: `.`, `...` and `a.b` stay valid, and `..` stays valid as a path.
    #[test]
    fn only_the_namespace_dot_dot_is_refused() {
        assert_eq!(
            Identifier::from_parts("..", "x"),
            Err(IdentifierError::Namespace)
        );
        assert_eq!(Identifier::parse("..:x"), Err(IdentifierError::Namespace));
        assert_eq!(Identifier::parse_namespaced("..:x"), None);
        for namespace in [".", "...", "a.b", "..a", "a..", "minecraft"] {
            let id = Identifier::from_parts(namespace, "x").expect(namespace);
            assert_eq!(id.to_string(), format!("{namespace}:x"));
        }
        assert_eq!(
            Identifier::from_parts("a", "..").map(|id| id.to_string()),
            Ok("a:..".to_owned())
        );
    }

    /// Non-ASCII text is refused in the namespace and in the path, never hashed. A valid
    /// identifier hashes as vanilla's `31 * namespace.hashCode() + path.hashCode()`.
    #[test]
    fn only_ascii_reaches_the_hash() {
        for text in ["\u{e9}", "a\u{1F600}", "\u{0101}", "\u{ff}"] {
            assert_eq!(
                Identifier::from_parts(text, "x"),
                Err(IdentifierError::Namespace)
            );
            assert_eq!(
                Identifier::from_parts("x", text),
                Err(IdentifierError::Path)
            );
        }
        assert_eq!(
            Identifier::parse("minecraft:stone").unwrap().hash_code(),
            182_203_480
        );
    }

    #[test]
    fn java_key_hashes_namespaces_and_orders_paths_first() {
        let id = |name| Identifier::parse(name).unwrap();
        let later_namespace = id("z:a");
        let earlier_namespace = id("a:z");
        assert_eq!(later_namespace.java_hash(), 3_879); // 31 * 'z' + 'a'
        assert_eq!(
            later_namespace.comparable_compare(&earlier_namespace),
            Some(Ordering::Less)
        );
        assert_eq!(
            later_namespace.tie_break(&earlier_namespace),
            Ordering::Less
        );
        assert_eq!(later_namespace.tie_break(&id("a:a")), Ordering::Greater);
    }

    /// A character outside `[a-z0-9_.-]` is refused in the namespace; `/` only in the path.
    #[test]
    fn invalid_characters_are_still_refused() {
        assert_eq!(
            Identifier::from_parts("A", "x"),
            Err(IdentifierError::Namespace)
        );
        assert_eq!(
            Identifier::from_parts("a/b", "x"),
            Err(IdentifierError::Namespace)
        );
        assert_eq!(
            Identifier::from_parts("a", "x y"),
            Err(IdentifierError::Path)
        );
        assert!(Identifier::from_parts("a", "x/y").is_ok());
    }
}
