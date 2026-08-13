/// A compiled glob pattern used to match file paths.
///
/// Supports `*` (any characters except `/`), `**` (any characters including
/// `/`), and `?` (any single character except `/`).
#[derive(Debug, Clone)]
pub struct Pattern {
    raw: String,
    has_slash: bool,
}

impl Pattern {
    /// Compiles `raw`. Patterns containing a `/` match the whole path, all
    /// others only the file name.
    pub fn new(raw: &str) -> Self {
        Self {
            raw: raw.to_string(),
            has_slash: raw.contains('/'),
        }
    }

    /// Whether this pattern is matched against the full path instead of the
    /// file name.
    pub const fn wants_full_path(&self) -> bool {
        self.has_slash
    }

    /// Returns `true` if the pattern matches `text`, which must be the file
    /// name or the `/`-separated full path as per [`wants_full_path`].
    ///
    /// [`wants_full_path`]: Pattern::wants_full_path
    pub fn matches(&self, text: &str) -> bool {
        // All meta-characters are ASCII, so bytes are enough and nothing is
        // allocated.
        glob_recurse(self.raw.as_bytes(), text.as_bytes())
    }
}

fn glob_recurse(p: &[u8], s: &[u8]) -> bool {
    if p.is_empty() {
        return s.is_empty();
    }

    // `**` matches any sequence including `/`.
    if p.starts_with(b"**") {
        let rest = &p[2..];
        return (0..=s.len()).any(|i| glob_recurse(rest, &s[i..]));
    }

    // `*` matches any sequence that does not cross a `/`.
    if p[0] == b'*' {
        let rest = &p[1..];
        let mut i = 0;
        loop {
            if glob_recurse(rest, &s[i..]) {
                return true;
            }
            if i == s.len() || s[i] == b'/' {
                return false;
            }
            i += 1;
        }
    }

    if s.is_empty() {
        return false;
    }

    // `?` matches any single character except `/`.
    if p[0] == b'?' && s[0] != b'/' {
        return glob_recurse(&p[1..], &s[1..]);
    }

    p[0] == s[0] && glob_recurse(&p[1..], &s[1..])
}

#[cfg(test)]
mod tests {
    use super::Pattern;

    fn m(pattern: &str, text: &str) -> bool {
        Pattern::new(pattern).matches(text)
    }

    #[test]
    fn star_matches_within_a_segment() {
        assert!(m("*.pdf", "report.pdf"));
        assert!(m("*.pdf", ".pdf"));
        assert!(!m("*.pdf", "report.pdf.part"));
        assert!(!m("*.pdf", "a/report.pdf"));
    }

    #[test]
    fn question_mark_matches_one_character() {
        assert!(m("img?.png", "img1.png"));
        assert!(!m("img?.png", "img10.png"));
        assert!(!m("a?b", "a/b"));
    }

    #[test]
    fn double_star_crosses_separators() {
        assert!(m("/home/*/docs/**", "/home/u/docs/a/b.txt"));
        assert!(m("**/*.txt", "a/b/c.txt"));
        assert!(!m("/home/*/docs/*", "/home/u/docs/a/b.txt"));
    }

    #[test]
    fn literal_must_match_exactly() {
        assert!(m("notes.txt", "notes.txt"));
        assert!(!m("notes.txt", "notes.txt2"));
        assert!(!m("notes.txt", "xnotes.txt"));
        assert!(!m("", "a"));
        assert!(m("", ""));
    }

    #[test]
    fn slash_decides_what_is_matched() {
        assert!(!Pattern::new("*.pdf").wants_full_path());
        assert!(Pattern::new("/tmp/*.pdf").wants_full_path());
        assert!(Pattern::new("sub/*.pdf").wants_full_path());
    }

    #[test]
    fn unicode_names_match() {
        assert!(m("*.pdf", "résumé 日本.pdf"));
        assert!(m("?.txt", "a.txt"));
    }
}
