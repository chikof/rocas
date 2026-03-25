/// A compiled glob pattern used to match file paths.
///
/// Supports `*` (any characters except `/`), `**` (any characters including
/// `/`), and `?` (any single character except `/`).
#[derive(Debug, Clone)]
pub struct Pattern {
    pub raw: String,
}

impl Pattern {
    /// Creates a new `Pattern` from the given raw glob string.
    pub fn new(raw: &str) -> Self {
        Self { raw: raw.to_string() }
    }

    /// Returns `true` if this pattern matches `path`.
    ///
    /// When the pattern contains a `/` it is matched against the full path;
    /// otherwise it is matched against the filename component only.
    pub fn matches(&self, path: &str) -> bool {
        // Work with bytes: all glob meta-characters (`*`, `?`, `/`) are ASCII,
        // so byte-level comparison is correct and avoids Vec<char> allocations.
        glob_match(self.raw.as_bytes(), path.as_bytes())
    }
}

/// Simple glob matching supporting `*`, `**`, and `?`.
///
/// Operates on byte slices; all meta-characters are ASCII so no UTF-8
/// decoding is required.
fn glob_match(pattern: &[u8], input: &[u8]) -> bool {
    glob_recurse(pattern, input)
}

/// Recursive glob helper.
fn glob_recurse(p: &[u8], s: &[u8]) -> bool {
    // Both exhausted → full match.
    if p.is_empty() {
        return s.is_empty();
    }

    // Double-star (`**`): matches any sequence including `/`.
    if p.starts_with(b"**") {
        let rest = &p[2..];
        // Try matching `**` against 0 to len(s) characters.
        for i in 0..=s.len() {
            if glob_recurse(rest, &s[i..]) {
                return true;
            }
        }
        return false;
    }

    // Single-star (`*`): matches any sequence that does not cross a `/`.
    if p[0] == b'*' {
        let rest = &p[1..];
        let mut i = 0;
        loop {
            if glob_recurse(rest, &s[i..]) {
                return true;
            }
            if i == s.len() || s[i] == b'/' {
                break;
            }
            i += 1;
        }
        return false;
    }

    // Pattern has more characters but the string is exhausted.
    if s.is_empty() {
        return false;
    }

    // `?` matches any single character except `/`.
    if p[0] == b'?' && s[0] != b'/' {
        return glob_recurse(&p[1..], &s[1..]);
    }

    // Literal match.
    if p[0] == s[0] {
        return glob_recurse(&p[1..], &s[1..]);
    }

    false
}
