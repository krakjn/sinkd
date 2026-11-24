//! Last-match ignore lines. `.gitignore` files first, then the anchor list.
//! A later `!` puts a child back, even when a parent directory was excluded.

use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
struct Pat {
    base: String,
    negate: bool,
    dir_only: bool,
    anchored: bool,
    body: String,
}

#[derive(Debug, Clone, Default)]
pub struct Rules {
    pats: Vec<Pat>,
}

impl Rules {
    #[must_use]
    pub fn load(root: &Path, extra: &[String]) -> Self {
        let mut pats = Vec::new();
        collect(root, root, &mut pats);
        for line in extra {
            if let Some(pat) = parse_line(line, "") {
                pats.push(pat);
            }
        }
        Self { pats }
    }

    /// True when the last matching pattern excludes `rel` (relative, `/` separators).
    #[must_use]
    pub fn ignored(&self, rel: &str, is_dir: bool) -> bool {
        let rel = rel.trim_matches('/');
        if rel.is_empty() {
            return false;
        }
        let mut excluded = false;
        for pat in &self.pats {
            if pat.hits(rel, is_dir) {
                excluded = !pat.negate;
            }
        }
        excluded
    }

    /// An ignored directory still needs a walk when a later `!` could match inside it.
    #[must_use]
    pub fn descend(&self, rel_dir: &str) -> bool {
        let rel_dir = rel_dir.trim_matches('/');
        if !self.ignored(rel_dir, true) {
            return true;
        }
        self.pats.iter().any(|pat| pat.negate && pat.might_include_under(rel_dir))
    }
}

fn collect(root: &Path, dir: &Path, pats: &mut Vec<Pat>) {
    let base = dir
        .strip_prefix(root)
        .unwrap_or(Path::new(""))
        .to_string_lossy()
        .replace('\\', "/");
    if let Ok(text) = fs::read_to_string(dir.join(".gitignore")) {
        for line in text.lines() {
            if let Some(pat) = parse_line(line, &base) {
                pats.push(pat);
            }
        }
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut kids = Vec::new();
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() && entry.file_name() != ".git" {
            kids.push(entry.path());
        }
    }
    kids.sort();
    for kid in kids {
        collect(root, &kid, pats);
    }
}

fn parse_line(line: &str, base: &str) -> Option<Pat> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (negate, rest) = line.strip_prefix('!').map_or((false, line), |rest| (true, rest));
    let dir_only = rest.ends_with('/');
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() {
        return None;
    }
    let anchored = rest.starts_with('/') || rest.contains('/') || rest.contains("**");
    Some(Pat {
        base: base.trim_matches('/').to_string(),
        negate,
        dir_only,
        anchored,
        body: rest.trim_start_matches('/').to_string(),
    })
}

impl Pat {
    fn hits(&self, rel: &str, is_dir: bool) -> bool {
        let Some(rest) = under_base(&self.base, rel) else { return false };
        if rest.is_empty() {
            return false;
        }
        if self.anchored {
            let pat: Vec<&str> = self.body.split('/').filter(|s| !s.is_empty()).collect();
            let path: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
            return prefix_hit(&pat, &path, self.dir_only, is_dir);
        }
        let name = self.body.as_str();
        let segs: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
        for (i, seg) in segs.iter().enumerate() {
            if !glob_seg(name, seg) {
                continue;
            }
            let last = i + 1 == segs.len();
            if last {
                return !self.dir_only || is_dir;
            }
            if self.dir_only {
                return true;
            }
        }
        false
    }

    fn might_include_under(&self, rel_dir: &str) -> bool {
        let full = if self.base.is_empty() {
            self.body.clone()
        } else {
            format!("{}/{}", self.base, self.body)
        };
        let literal: String = full.split("**").next().unwrap_or("").split('*').next().unwrap_or("").trim_matches('/').to_string();
        if literal.is_empty() {
            return true;
        }
        literal == rel_dir || literal.starts_with(&format!("{rel_dir}/")) || rel_dir.starts_with(&format!("{literal}/"))
    }
}

fn under_base<'a>(base: &str, rel: &'a str) -> Option<&'a str> {
    if base.is_empty() {
        return Some(rel);
    }
    if rel == base {
        return Some("");
    }
    rel.strip_prefix(base)?.strip_prefix('/')
}

fn prefix_hit(pat: &[&str], path: &[&str], dir_only: bool, is_dir: bool) -> bool {
    for len in 0..=path.len() {
        if !segs(pat, &path[..len]) {
            continue;
        }
        if len < path.len() {
            if dir_only {
                return true;
            }
            continue;
        }
        return !dir_only || is_dir;
    }
    false
}

fn segs(pat: &[&str], path: &[&str]) -> bool {
    if pat.is_empty() {
        return path.is_empty();
    }
    if pat[0] == "**" {
        return segs(&pat[1..], path) || (!path.is_empty() && segs(pat, &path[1..]));
    }
    if path.is_empty() || !glob_seg(pat[0], path[0]) {
        return false;
    }
    segs(&pat[1..], &path[1..])
}

fn glob_seg(pat: &str, seg: &str) -> bool {
    fn rec(pat: &[u8], seg: &[u8]) -> bool {
        match (pat.first(), seg.first()) {
            (None, None) => true,
            (Some(b'*'), _) => rec(&pat[1..], seg) || (!seg.is_empty() && rec(pat, &seg[1..])),
            (Some(b'?'), Some(_)) => rec(&pat[1..], &seg[1..]),
            (Some(p), Some(s)) if p == s => rec(&pat[1..], &seg[1..]),
            _ => false,
        }
    }
    rec(pat.as_bytes(), seg.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(lines: &[&str]) -> Rules {
        Rules {
            pats: lines.iter().filter_map(|line| parse_line(line, "")).collect(),
        }
    }

    #[test]
    fn later_bang_puts_the_child_back() {
        let rules = rules(&["something/", "!something/thing/"]);
        assert!(rules.ignored("something", true));
        assert!(rules.ignored("something/other", false));
        assert!(!rules.ignored("something/thing", true));
        assert!(!rules.ignored("something/thing/a.txt", false));
        assert!(rules.descend("something"));
        assert!(!rules.descend("something/other"));
    }

    #[test]
    fn gitignore_then_anchor_list() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".gitignore"), "something/\n").unwrap();
        fs::create_dir(tmp.path().join("something")).unwrap();
        let rules = Rules::load(tmp.path(), &["!something/thing/".into()]);
        assert!(rules.ignored("something/other", false));
        assert!(!rules.ignored("something/thing/a.txt", false));
    }

    #[test]
    fn star_matches_a_name() {
        let rules = rules(&["*.log", "!keep.log"]);
        assert!(rules.ignored("a/debug.log", false));
        assert!(!rules.ignored("keep.log", false));
    }
}
