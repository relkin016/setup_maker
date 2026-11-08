//! Tiny case-insensitive wildcard matcher (`*`, `?`) used for file sources,
//! excludes and archive names.

pub fn matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub fn has_wildcard(s: &str) -> bool {
    s.contains('*') || s.contains('?')
}

#[cfg(test)]
mod tests {
    #[test]
    fn w() {
        assert!(super::matches("*.arc", "DATA1.ARC"));
        assert!(super::matches("data?.7z", "data2.7z"));
        assert!(!super::matches("*.arc", "a.7z"));
        assert!(super::matches("*", "x"));
        assert!(super::matches("CODMW_*.arc", "codmw_2.arc"));
    }
}
