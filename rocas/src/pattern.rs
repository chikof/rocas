/// A compiled glob pattern used to match file paths.
///
/// Supports `*` (any characters except `/`), `**` (any characters including
/// `/`), and `?` (any single byte except `/`). There are no character
/// classes.
///
/// Matching works on UTF-8 bytes, so `?` stands for one byte, not one
/// character: a character outside ASCII needs one `?` per byte (`??` matches
/// `é`). `*` and `**` are unaffected, as every byte of a multi-byte
/// character is above `0x7F` and never equals `/`.
///
/// Matching never backtracks exponentially: it takes at most pattern length
/// times text length steps, and a `**` followed by `*` can add one more factor
/// of the text length.
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
        glob(self.raw.as_bytes(), text.as_bytes())
    }
}

/// Iterative matcher that remembers where the last wildcard started instead
/// of recursing, so a failed match never retries the same state.
///
/// After a mismatch it widens the last `*` by one byte, unless that byte is
/// `/`. A `*` blocked that way cannot be helped by an earlier `*` (which
/// cannot cross the `/` either), but an earlier `**` can, so the last `**` is
/// the second resume point. Earlier ones are not needed: the text between two
/// `**` is free, so the leftmost match of the segment between them is always
/// the best one.
fn glob(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut ti) = (0, 0);
    // (pattern index after the wildcard, text index its expansion ends at)
    let mut star: Option<(usize, usize)> = None;
    let mut deep: Option<(usize, usize)> = None;

    while pi < p.len() || ti < s.len() {
        match p.get(pi) {
            Some(b'*') => {
                if p.get(pi + 1) == Some(&b'*') {
                    pi += 2;
                    deep = Some((pi, ti));
                    star = None;
                } else {
                    pi += 1;
                    star = Some((pi, ti));
                }
                continue;
            },
            Some(&c) if ti < s.len() && (c == s[ti] || (c == b'?' && s[ti] != b'/')) => {
                pi += 1;
                ti += 1;
                continue;
            },
            _ => {},
        }

        if let Some((resume, end)) = star.as_mut()
            && *end < s.len()
            && s[*end] != b'/'
        {
            *end += 1;
            (pi, ti) = (*resume, *end);
        } else if let Some((resume, end)) = deep.as_mut()
            && *end < s.len()
        {
            *end += 1;
            (pi, ti) = (*resume, *end);
            star = None;
        } else {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

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

    #[test]
    fn question_mark_counts_bytes_not_characters() {
        assert!(m("?.txt", "a.txt"));
        assert!(!m("?.txt", "é.txt"));
        assert!(m("??.txt", "é.txt"));
        assert!(!m("??.txt", "a.txt"));
        assert!(m("??.txt", "ab.txt"));
        assert!(m("*.txt", "é.txt"));
    }

    #[test]
    fn empty_pattern_and_text() {
        assert!(m("", ""));
        assert!(!m("", "a"));
        assert!(!m("a", ""));
        assert!(m("*", ""));
        assert!(m("**", ""));
        assert!(m("***", ""));
        assert!(!m("?", ""));
        assert!(m("*", "abc"));
        assert!(!m("*", "a/b"));
        assert!(m("**", "a/b"));
    }

    #[test]
    fn a_star_blocked_by_a_slash_falls_back_to_an_earlier_double_star() {
        assert!(m("**x*y", "axb/xy"));
        assert!(m("**/a*b", "x/y/a--b"));
        assert!(!m("**/a*b", "x/y/a-/-b"));
        assert!(m("/home/*/docs/**/*.pdf", "/home/u/docs/a/b/c.pdf"));
        assert!(!m("/home/*/docs/**/*.pdf", "/home/u/v/docs/c.pdf"));
    }

    #[test]
    fn many_wildcards_do_not_blow_up() {
        let name = "a".repeat(4096);
        let text = name.as_str();
        for pattern in ["*a*a*a*a*a*b", "**a**a**a**a**a**b", "*a?*a?*a?*a?*a?*b"] {
            let start = Instant::now();
            assert!(!m(pattern, text), "{pattern}");
            assert!(
                start.elapsed() < Duration::from_millis(10),
                "{pattern} took {:?}",
                start.elapsed()
            );
        }
        let start = Instant::now();
        assert!(m("*a*a*a*a*a*a", text));
        assert!(start.elapsed() < Duration::from_millis(10));
    }

    /// The previous recursive matcher, kept as the specification.
    fn reference(p: &[u8], s: &[u8]) -> bool {
        if p.is_empty() {
            return s.is_empty();
        }
        if p.starts_with(b"**") {
            return (0..=s.len()).any(|i| reference(&p[2..], &s[i..]));
        }
        if p[0] == b'*' {
            let mut i = 0;
            loop {
                if reference(&p[1..], &s[i..]) {
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
        if p[0] == b'?' && s[0] != b'/' {
            return reference(&p[1..], &s[1..]);
        }
        p[0] == s[0] && reference(&p[1..], &s[1..])
    }

    /// Every string over `alphabet` up to `max` bytes long.
    fn strings(alphabet: &[u8], max: usize) -> Vec<Vec<u8>> {
        let mut all = vec![Vec::new()];
        let mut layer = vec![Vec::new()];
        for _ in 0..max {
            layer = layer
                .iter()
                .flat_map(|s| {
                    alphabet.iter().map(move |&c| {
                        let mut next = s.clone();
                        next.push(c);
                        next
                    })
                })
                .collect();
            all.extend(layer.iter().cloned());
        }
        all
    }

    #[test]
    fn agrees_with_the_recursive_matcher_on_every_small_input() {
        let patterns = strings(b"ab/*?", 6);
        let texts = strings(b"ab/", 6);
        for p in &patterns {
            for s in &texts {
                assert_eq!(
                    super::glob(p, s),
                    reference(p, s),
                    "pattern {:?} text {:?}",
                    String::from_utf8_lossy(p),
                    String::from_utf8_lossy(s)
                );
            }
        }
    }
}
