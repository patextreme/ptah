//! Project discovery and the pesde [`Project`] handle.
//!
//! A package project is rooted at the nearest ancestor `.ptah` of the
//! invocation directory that carries `config.toml` or `pesde.toml` —
//! a generalization of the agent-registry discovery (which only looks
//! for `config.toml`), so package commands work from any subdirectory
//! inside a project and stop at the filesystem root.

use std::path::{Path, PathBuf};

use crate::Error;

/// A ptah project with its embedded Pesde engine handle.
#[derive(Debug, Clone)]
pub struct PackageProject {
    /// The project root: the directory containing `.ptah/`.
    root: PathBuf,
    /// The Pesde project rooted at `<root>/.ptah`.
    pesde: pesde::Project,
}

impl PackageProject {
    /// Open the project rooted at `root` (the directory containing
    /// `.ptah/`). Per-project caches: general data under
    /// `.ptah/.pesde/.data` (index clones, git dependency repos) and
    /// the content-addressable store under `.ptah/.pesde/.cas` — the
    /// CAS sits on the same filesystem as the packages it hard-links
    /// into, by construction. The dot-prefixed names are deliberate:
    /// pesde's `remove_unused` sweep treats `<package>/.pesde` as its
    /// scripts-link folder and deletes entries whose names parse as
    /// Pesde aliases but aren't graph aliases — `data`/`cas` parse as
    /// aliases and would be wiped on every install; `.data`/`.cas`
    /// don't parse and survive. No workspace: ptah projects are a
    /// flat single-package layout.
    pub fn open(root: &Path) -> Self {
        let ptah_dir = root.join(".ptah");
        let pesde = pesde::Project::new(
            &ptah_dir,
            None::<&Path>,
            ptah_dir.join(".pesde").join(".data"),
            ptah_dir.join(".pesde").join(".cas"),
            pesde::AuthConfig::new(),
        );
        Self {
            root: root.to_path_buf(),
            pesde,
        }
    }

    /// Discover the project for an invocation directory: the nearest
    /// ancestor (starting at `start` itself) whose `.ptah/` contains
    /// `config.toml` or `pesde.toml`, then open it. Errors when no
    /// such directory exists up to the filesystem root.
    pub fn discover(start: &Path) -> Result<Self, Error> {
        find_project_root(start).map_or_else(
            || Err(Error::NoProject {
                searched_from: start.to_path_buf(),
            }),
            |root| Ok(Self::open(&root)),
        )
    }

    /// The project root (the directory containing `.ptah/`).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `.ptah/` directory itself (Pesde's package dir).
    pub fn ptah_dir(&self) -> PathBuf {
        self.root.join(".ptah")
    }

    /// The underlying Pesde project handle.
    pub fn pesde(&self) -> &pesde::Project {
        &self.pesde
    }
}

/// Nearest ancestor of `start` (inclusive) whose `.ptah/` contains
/// `config.toml` or `pesde.toml`. Stops at the filesystem root — the
/// walk-up ends where `Path::parent` does.
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut dir: &Path = start;
    loop {
        let ptah = dir.join(".ptah");
        if ptah.join("config.toml").is_file() || ptah.join("pesde.toml").is_file() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".ptah")).unwrap();
        std::fs::write(dir.path().join(".ptah/config.toml"), "").unwrap();
        let _ = name;
        dir
    }

    #[test]
    fn finds_the_project_at_the_invocation_root() {
        let dir = fixture("root");
        let project = PackageProject::discover(dir.path()).unwrap();
        assert_eq!(project.root(), dir.path());
        assert_eq!(project.ptah_dir(), dir.path().join(".ptah"));
    }

    #[test]
    fn finds_the_project_from_a_nested_directory() {
        let dir = fixture("nested");
        let nested = dir.path().join(".ptah/workflows/openspec");
        std::fs::create_dir_all(&nested).unwrap();
        let project = PackageProject::discover(&nested).unwrap();
        assert_eq!(project.root(), dir.path());
    }

    #[test]
    fn no_project_above_is_an_error_naming_the_search() {
        let dir = tempfile::tempdir().unwrap();
        let err = PackageProject::discover(dir.path()).unwrap_err();
        assert!(matches!(err, Error::NoProject { .. }), "{err}");
        assert!(err.is_usage(), "no-project is a usage error (exit 2)");
        let msg = err.to_string();
        let searched = dir.path().display().to_string();
        assert!(msg.contains(&searched), "names the searched-from dir: {msg}");
        assert!(
            msg.contains("config.toml") && msg.contains("pesde.toml"),
            "names what was searched for: {msg}"
        );
    }

    #[test]
    fn a_pesde_manifest_alone_marks_a_project() {
        // `.ptah/pesde.toml` without `config.toml` is a valid project
        // root for package commands (a package-only project).
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".ptah")).unwrap();
        std::fs::write(dir.path().join(".ptah/pesde.toml"), "").unwrap();
        let project = PackageProject::discover(dir.path()).unwrap();
        assert_eq!(project.root(), dir.path());
    }

    #[test]
    fn config_discovery_still_finds_registry_only_projects() {
        // The registry discovery case (config.toml only, no manifest)
        // is also a package project root — discovery is the union.
        let dir = fixture("registry-only");
        let project = PackageProject::discover(dir.path()).unwrap();
        assert_eq!(project.root(), dir.path());
    }

    #[test]
    fn caches_live_under_ptah_dot_pesde() {
        let dir = fixture("caches");
        let project = PackageProject::discover(dir.path()).unwrap();
        let pesde = project.pesde();
        assert_eq!(
            pesde.package_dir(),
            dir.path().join(".ptah"),
            "package dir is .ptah itself"
        );
        assert_eq!(
            pesde.data_dir(),
            dir.path().join(".ptah/.pesde/.data"),
            "index/git caches are per-project"
        );
        assert_eq!(
            pesde.cas_dir(),
            dir.path().join(".ptah/.pesde/.cas"),
            "CAS sits beside the packages it links into"
        );
    }
}
