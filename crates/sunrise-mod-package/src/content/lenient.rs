//! The one place a content file's comments are skipped, as Gson's lenient reader does (vanilla reads data
//! files with it): shared by the port's carry and the reference check.

/// JSON text without the comments Gson's lenient reader skips: `//` and
/// `#` to the end of the line, and `/* ... */`, outside strings.
pub fn without_comments(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut string = false;
    while i < bytes.len() {
        let b = bytes[i];
        if string {
            out.push(b);
            if b == b'\\' {
                out.extend(bytes.get(i + 1));
                i += 1;
            } else if b == b'"' {
                string = false;
            }
            i += 1;
            continue;
        }
        match (b, bytes.get(i + 1)) {
            (b'"', _) => {
                string = true;
                out.push(b);
                i += 1;
            }
            (b'/', Some(b'/')) | (b'#', _) => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            (b'/', Some(b'*')) => {
                i += 2;
                while i < bytes.len() && !bytes[i..].starts_with(b"*/") {
                    i += 1;
                }
                i += 2;
                out.push(b' ');
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}
