#[derive(Debug, Clone)]
pub struct Pattern {
    pub raw: String,
}

impl Pattern {
    /// Creates a new Pattern from the given raw string.
    pub fn new(raw: &str) -> Self {
        Self { raw: raw.to_string() }
    }

    /// Matches the pattern against the given path. If the pattern contains a
    /// '/', it matches against the full path; otherwise, it matches against
    /// just the filename.
    pub fn matches(&self, path: &str) -> bool {
        glob_match(&self.raw, path)
    }
}

/// Simple glob matching supporting '*', '**', and '?'
fn glob_match(pattern: &str, input: &str) -> bool {
    let p: Vec<char> = pattern
        .chars()
        .collect();
    let s: Vec<char> = input
        .chars()
        .collect();

    glob_recurse(&p, &s, 0, 0)
}

/// Recursive helper for glob matching
fn glob_recurse(p: &[char], s: &[char], pi: usize, si: usize) -> bool {
    // Both exhausted: full match
    if pi == p.len() && si == s.len() {
        return true;
    }

    // Pattern exhausted but string remains
    if pi == p.len() {
        return false;
    }

    // Double star (**): matches anything including slashes
    if pi + 1 < p.len() && p[pi] == '*' && p[pi + 1] == '*' {
        // Try matching ** against 0 or more characters
        for i in si..=s.len() {
            if glob_recurse(p, s, pi + 2, i) {
                return true;
            }
        }
        return false;
    }

    // Single star (*): matches anything except '/'
    if p[pi] == '*' {
        let mut i = si;
        while i <= s.len() {
            if glob_recurse(p, s, pi + 1, i) {
                return true;
            }
            if i < s.len() && s[i] == '/' {
                break; // single * can't cross directories
            }
            i += 1;
        }
        return false;
    }

    // String exhausted but pattern remains (and it's not a star)
    if si == s.len() {
        return false;
    }

    // '?' matches any single character except '/'
    if p[pi] == '?' && s[si] != '/' {
        return glob_recurse(p, s, pi + 1, si + 1);
    }

    // Literal match
    if p[pi] == s[si] {
        return glob_recurse(p, s, pi + 1, si + 1);
    }

    false
}
