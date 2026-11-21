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

/// Case-insensitive "natural" order: `patch2` < `patch10`.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let si = i;
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            let sj = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let na: String = a[si..i].iter().collect::<String>().trim_start_matches('0').to_string();
            let nb: String = b[sj..j].iter().collect::<String>().trim_start_matches('0').to_string();
            let o = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb)).then_with(|| (i - si).cmp(&(j - sj)));
            if o != Ordering::Equal {
                return o;
            }
        } else {
            if a[i] != b[j] {
                return a[i].cmp(&b[j]);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
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
        let mut v = vec!["patch10.zip", "Patch2.zip", "patch1.zip", "patch.zip", "patch02a.zip"];
        v.sort_by(|a, b| super::natural_cmp(a, b));
        assert_eq!(v, ["patch.zip", "patch1.zip", "Patch2.zip", "patch02a.zip", "patch10.zip"]);
    }
}
