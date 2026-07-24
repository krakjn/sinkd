use std::{
    env,
    fs,
    path::PathBuf,
};

/// Repo root: directory containing `pkg/debian/create.sh`.
pub fn find_repo_root() -> Option<PathBuf> {
    let mut starts: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = env::current_dir() {
        starts.push(cwd);
    }
    if let Ok(exe) = env::current_exe()
        && let Some(parent) = exe.parent()
    {
        starts.push(parent.to_path_buf());
    }
    for start in starts {
        let root = fs::canonicalize(&start).unwrap_or(start);
        let mut dir = root.as_path();
        loop {
            if dir.join("pkg/debian/create.sh").is_file() {
                return Some(dir.to_path_buf());
            }
            match dir.parent() {
                Some(p) => dir = p,
                None => break,
            }
        }
    }
    None
}

#[inline]
pub fn sanitize_spec_segment(name: &str) -> String {
    if name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return name.to_string();
    }
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn spec_output_dir(spec_name: &str) -> Result<PathBuf, String> {
    let repo = find_repo_root().ok_or("cannot locate repo root (expected pkg/debian/create.sh)")?;
    Ok(repo
        .join("scenario")
        .join("output")
        .join(sanitize_spec_segment(spec_name)))
}

pub fn ensure_spec_output_dir(spec_name: &str) -> Result<PathBuf, String> {
    let dir = spec_output_dir(spec_name)?;
    fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "failed to create scenario output dir '{}': {e}",
            dir.display()
        )
    })?;
    Ok(dir)
}

pub fn spec_workspace_dir(spec_name: &str) -> Result<PathBuf, String> {
    let dir = ensure_spec_output_dir(spec_name)?.join("workspace");
    fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "failed to create scenario workspace dir '{}': {e}",
            dir.display()
        )
    })?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::{find_repo_root, sanitize_spec_segment, spec_output_dir};

    #[test]
    fn find_repo_root_from_cargo_manifest() {
        assert!(find_repo_root().is_some());
    }

    #[test]
    fn sanitize_spec_segment_replaces_invalid_chars() {
        assert_eq!(sanitize_spec_segment("compose"), "compose");
        assert_eq!(sanitize_spec_segment("a/b"), "a_b");
    }

    #[test]
    fn spec_output_dir_under_scenario_output() {
        let dir = spec_output_dir("smoke").expect("repo root");
        assert!(dir.ends_with("scenario/output/smoke"));
    }
}
