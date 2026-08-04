use std::path::{Path, PathBuf};

pub const MANIFEST_FILE_NAME: &str = "devcroft.toml";

#[derive(Debug)]
pub struct NotFound;

/// Search `start` and its ancestors for `devcroft.toml`, stopping at the
/// filesystem root or a `.git` boundary, whichever comes first.
pub fn discover(start: &Path) -> Result<PathBuf, NotFound> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join(MANIFEST_FILE_NAME);
        if candidate.is_file() {
            return Ok(candidate);
        }
        if dir.join(".git").exists() {
            return Err(NotFound);
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return Err(NotFound),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "devcroft-discovery-test-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_manifest_in_ancestor() {
        let root = tempdir("ancestor");
        let deep = root.join("src/deep");
        fs::create_dir_all(&deep).unwrap();
        fs::write(root.join(MANIFEST_FILE_NAME), "").unwrap();

        let found = discover(&deep).unwrap();
        assert_eq!(found, root.join(MANIFEST_FILE_NAME));
    }

    #[test]
    fn stops_at_git_boundary_without_manifest() {
        let root = tempdir("git-boundary");
        let project = root.join("project");
        let deep = project.join("src");
        fs::create_dir_all(&deep).unwrap();
        fs::create_dir_all(project.join(".git")).unwrap();
        // A manifest above the .git boundary must not be found.
        fs::write(root.join(MANIFEST_FILE_NAME), "").unwrap();

        assert!(discover(&deep).is_err());
    }

    #[test]
    fn finds_manifest_at_git_boundary_itself() {
        let root = tempdir("git-boundary-hit");
        let project = root.join("project");
        let deep = project.join("src");
        fs::create_dir_all(&deep).unwrap();
        fs::create_dir_all(project.join(".git")).unwrap();
        fs::write(project.join(MANIFEST_FILE_NAME), "").unwrap();

        let found = discover(&deep).unwrap();
        assert_eq!(found, project.join(MANIFEST_FILE_NAME));
    }

    #[test]
    fn no_manifest_anywhere() {
        let root = tempdir("none");
        assert!(discover(&root).is_err());
    }
}
