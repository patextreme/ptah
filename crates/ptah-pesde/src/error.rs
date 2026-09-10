//! The adapter's error surface: one enum the CLI maps onto the package
//! exit-code contract — `usage` failures exit 2, everything else exits
//! 1 — with each variant naming the failing step or package the way
//! the spec's diagnostics require.

use std::path::PathBuf;

/// A package-management failure.
#[derive(Debug)]
pub enum Error {
    // ------------------------------------------------------------------
    // Usage class (the CLI exits 2): the invocation or the project
    // configuration is wrong; retrying unchanged cannot succeed.
    // ------------------------------------------------------------------
    /// No project found: no ancestor directory of the invocation point
    /// contains a `.ptah/` with `config.toml` or `pesde.toml`.
    NoProject { searched_from: PathBuf },
    /// The manifest is missing from the discovered project.
    ManifestMissing { ptah_dir: PathBuf },
    /// The manifest could not be parsed as a Pesde manifest.
    ManifestParse { source: String },
    /// A manifest edit could not be represented (malformed table
    /// structure where a dependency table was expected).
    ManifestEdit { source: String },
    /// The requested dependency spec or alias is not a valid form.
    InvalidSpec { source: String },
    /// `remove` was asked for an alias the manifest does not carry.
    UnknownAlias { alias: String },

    // ------------------------------------------------------------------
    // Operational class (the CLI exits 1): the request was well-formed;
    // the operation failed.
    // ------------------------------------------------------------------
    /// The manifest's target environment is not `luau` (ptah only
    /// manages Luau packages).
    UnsupportedTarget { found: String },
    /// `install --locked` with no lockfile present.
    LockfileMissing { ptah_dir: PathBuf },
    /// `install --locked` with a lockfile out of sync with the manifest
    /// (name, target, overrides, or direct dependencies changed).
    LockfileStale { reason: String },
    /// Building the dependency graph failed (conflicts, cycles,
    /// unsatisfiable ranges).
    Resolve { source: String },
    /// Refreshing or querying a package source failed (git clone/fetch,
    /// index read, registry request).
    Source { step: &'static str, source: String },
    /// Downloading and linking packages failed.
    Install { source: String },
    /// Reading or writing a ptah-managed file failed.
    Io { context: String, source: std::io::Error },
    /// Synchronizing the root `.luaurc` failed (unparseable existing
    /// file, unwritable).
    LuaurcSync { source: String },
}

impl Error {
    /// Whether this is a usage-class failure (CLI exit 2): a malformed
    /// invocation or unusable project configuration, as opposed to an
    /// operational failure (exit 1).
    pub fn is_usage(&self) -> bool {
        matches!(
            self,
            Error::NoProject { .. }
                | Error::ManifestMissing { .. }
                | Error::ManifestParse { .. }
                | Error::ManifestEdit { .. }
                | Error::InvalidSpec { .. }
                | Error::UnknownAlias { .. }
        )
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoProject { searched_from } => write!(
                f,
                "no ptah project found: searched for .ptah/config.toml or .ptah/pesde.toml \
                 in {} and every parent directory",
                searched_from.display()
            ),
            Error::ManifestMissing { ptah_dir } => write!(
                f,
                "no package manifest at {} (run `ptah init` to scaffold one)",
                ptah_dir.join("pesde.toml").display()
            ),
            Error::ManifestParse { source } => {
                write!(f, "cannot parse .ptah/pesde.toml: {source}")
            }
            Error::ManifestEdit { source } => {
                write!(f, "cannot edit .ptah/pesde.toml: {source}")
            }
            Error::InvalidSpec { source } => write!(f, "invalid package specification: {source}"),
            Error::UnknownAlias { alias } => {
                write!(f, "no dependency with alias `{alias}` in the manifest")
            }
            Error::UnsupportedTarget { found } => write!(
                f,
                "unsupported package target environment `{found}` \
                 (ptah manages `luau` packages only)"
            ),
            Error::LockfileMissing { ptah_dir } => write!(
                f,
                "the lockfile is missing: {} (--locked installs need an up-to-date lockfile)",
                ptah_dir.join("pesde.lock").display()
            ),
            Error::LockfileStale { reason } => write!(
                f,
                "the lockfile is out of sync with the manifest ({reason}); \
                 run `ptah package install` to update it"
            ),
            Error::Resolve { source } => write!(f, "dependency resolution failed: {source}"),
            Error::Source { step, source } => write!(f, "failed to {step}: {source}"),
            Error::Install { source } => write!(f, "failed to install packages: {source}"),
            Error::Io { context, source } => write!(f, "{context}: {source}"),
            Error::LuaurcSync { source } => write!(f, "failed to sync .luaurc: {source}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Error::Io {
            context: "package I/O failed".to_string(),
            source,
        }
    }
}

impl From<pesde::errors::ManifestReadError> for Error {
    fn from(e: pesde::errors::ManifestReadError) -> Self {
        match e {
            pesde::errors::ManifestReadError::Io(ref io)
                if io.kind() == std::io::ErrorKind::NotFound =>
            {
                // The caller knows the ptah dir; it is not carried by
                // the pesde error, so the path is filled by the caller
                // where it matters (discovery). Here it stays empty.
                Error::ManifestMissing {
                    ptah_dir: PathBuf::new(),
                }
            }
            other => Error::ManifestParse {
                source: other.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_classification_matches_the_exit_code_contract() {
        for err in [
            Error::NoProject {
                searched_from: PathBuf::from("/x"),
            },
            Error::ManifestMissing {
                ptah_dir: PathBuf::from("/x"),
            },
            Error::ManifestParse {
                source: "bad".into(),
            },
            Error::ManifestEdit {
                source: "bad".into(),
            },
            Error::InvalidSpec {
                source: "bad".into(),
            },
            Error::UnknownAlias {
                alias: "x".into(),
            },
        ] {
            assert!(err.is_usage(), "{err} must be usage-class (exit 2)");
        }
        for err in [
            Error::UnsupportedTarget {
                found: "roblox".into(),
            },
            Error::LockfileMissing {
                ptah_dir: PathBuf::from("/x"),
            },
            Error::LockfileStale {
                reason: "r".into(),
            },
            Error::Resolve {
                source: "s".into(),
            },
            Error::Source {
                step: "refresh source",
                source: "s".into(),
            },
            Error::Install {
                source: "s".into(),
            },
            Error::Io {
                context: "c".into(),
                source: std::io::Error::other("x"),
            },
            Error::LuaurcSync {
                source: "s".into(),
            },
        ] {
            assert!(!err.is_usage(), "{err} must be operational (exit 1)");
        }
    }

    #[test]
    fn diagnostics_name_the_failing_thing() {
        let e = Error::UnknownAlias {
            alias: "hello".into(),
        };
        assert!(e.to_string().contains("hello"), "{}", e);
        let e = Error::UnsupportedTarget {
            found: "roblox".into(),
        };
        assert!(e.to_string().contains("roblox"), "{}", e);
        let e = Error::NoProject {
            searched_from: PathBuf::from("/tmp/here"),
        };
        assert!(e.to_string().contains("/tmp/here"), "{}", e);
        assert!(e.to_string().contains(".ptah/pesde.toml"), "{}", e);
    }
}
