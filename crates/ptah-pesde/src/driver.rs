//! The package operation driver: `install`, `update`, `add`, `remove`
//! — the thin orchestration layer Pesde ships only as CLI code
//! (~200 lines over public types), re-implemented here so ptah owns
//! the semantics: ptah-style errors, the `--locked` staleness
//! contract, lockfile writing, and the default-index materialization
//! around library resolution passes.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::str::FromStr as _;

use pesde::download_and_link::{DownloadAndLinkOptions, InstallDependenciesMode};
use pesde::lockfile::Lockfile;
use pesde::manifest::overrides::OverrideSpecifier;
use pesde::manifest::{Alias, Manifest};
use pesde::names::PackageName;
use pesde::source::git::specifier::GitDependencySpecifier;
use pesde::source::path::specifier::PathDependencySpecifier;
use pesde::source::pesde::specifier::PesdeDependencySpecifier;
use pesde::source::specifiers::DependencySpecifiers;
use pesde::source::traits::PackageSource as _;
use pesde::source::traits::{RefreshOptions, ResolveOptions};
use pesde::RefreshedSources;
use semver::VersionReq;

use crate::manifest::{self, DependencyEntry};
use crate::{Error, PackageProject};

/// Concurrency of registry downloads inside one install (pesde's own
/// default).
const NETWORK_CONCURRENCY: usize = 16;

/// What one full package operation did, for the CLI's stage lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    /// Packages in the resolved dependency graph.
    pub packages: usize,
    /// Whether the lockfile was rewritten (bytes changed; an
    /// up-to-date install rewrites nothing).
    pub lockfile_written: bool,
}

/// `install`/`update` mode knobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallOptions {
    /// `install --locked`: install exactly the lockfile's versions,
    /// failing when it is missing or stale.
    pub locked: bool,
    /// `update`: re-resolve every dependency from scratch (ignore the
    /// old graph's pinned versions for selection).
    pub fresh: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            locked: false,
            fresh: false,
        }
    }
}

/// The result of comparing the manifest against the lockfile.
#[derive(Debug)]
pub enum LockfileCheck {
    /// Present and in sync; carries the lockfile.
    Fresh(Box<Lockfile>),
    /// No lockfile on disk.
    Missing,
    /// Out of sync; carries the reason for the diagnostic.
    Stale { reason: String },
}

/// Run one full package operation: resolve (reusing the lockfile's
/// pinned versions unless `fresh`), download and link into
/// `luau_packages/`, and write the lockfile when its contents changed
/// (an up-to-date install changes no file). With `locked`, resolve is
/// only allowed to reuse the lockfile — missing or stale fails before
/// anything touches the network.
pub async fn install(
    project: &PackageProject,
    client: &reqwest::Client,
    opts: &InstallOptions,
) -> Result<InstallOutcome, Error> {
    let manifest = manifest::deser_manifest(project).await?;
    manifest::require_luau_target(&manifest)?;

    let check = check_lockfile(project, &manifest).await?;
    let previous = if opts.locked {
        match check {
            LockfileCheck::Fresh(lockfile) => Some(lockfile),
            LockfileCheck::Missing { .. } => {
                return Err(Error::LockfileMissing {
                    ptah_dir: project.ptah_dir(),
                })
            }
            LockfileCheck::Stale { reason } => {
                return Err(Error::LockfileStale { reason })
            }
        }
    } else {
        // Reuse the lockfile only when it is structurally compatible;
        // otherwise resolve fresh and force a relink (orphaned
        // container folders from a changed target/override set would
        // otherwise linger) — mirroring pesde's own install driver.
        match check {
            LockfileCheck::Fresh(lockfile) => Some(lockfile),
            _ => None,
        }
    };

    with_default_index(project, async |project| {
        let old_graph = previous
            .as_ref()
            .filter(|_| !opts.fresh)
            .map(|l| l.graph.clone());

        let refreshed = RefreshedSources::new();
        let graph = Box::pin(project.pesde().dependency_graph(
            old_graph.as_ref(),
            refreshed.clone(),
            false,
        ))
        .await
        .map_err(|e| Error::Resolve {
            source: error_chain(e.as_ref()),
        })?;

        let downloaded = Box::pin(project.pesde().download_and_link(
            &graph,
            DownloadAndLinkOptions::<(), ()>::new(client.clone())
                .refreshed_sources(refreshed.clone())
                .install_dependencies_mode(InstallDependenciesMode::All)
                .network_concurrency(NonZeroUsize::new(NETWORK_CONCURRENCY).expect("nonzero")),
        ))
        .await
        .map_err(|e| Error::Install {
            source: error_chain(&e),
        })?;
        let _ = downloaded;

        let lockfile = Lockfile {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            target: manifest.target.kind(),
            overrides: resolve_overrides(&manifest)?,
            graph: graph.clone(),
            workspace: BTreeMap::new(),
        };

        let new_text = lockfile_text(&lockfile)?;
        let lockfile_path = project.ptah_dir().join("pesde.lock");
        let unchanged = std::fs::read(&lockfile_path)
            .is_ok_and(|existing| existing == new_text.as_bytes());
        if !unchanged {
            project
                .pesde()
                .write_lockfile(&lockfile)
                .await
                .map_err(|e| Error::Io {
                    context: format!("cannot write {}", lockfile_path.display()),
                    source: std::io::Error::other(e.to_string()),
                })?;
        }

        Ok(InstallOutcome {
            packages: graph.len(),
            lockfile_written: !unchanged,
        })
    })
    .await
}

/// `ptah package update`: a full install with fresh resolution.
pub async fn update(
    project: &PackageProject,
    client: &reqwest::Client,
) -> Result<InstallOutcome, Error> {
    install(
        project,
        client,
        &InstallOptions {
            locked: false,
            fresh: true,
        },
    )
    .await
}

/// Where an added package comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddSource {
    /// A registry package `scope/name[@version-req]`.
    Registry {
        name: String,
        version: Option<String>,
    },
    /// A git repository, optionally pinned to a rev and subdirectory.
    Git {
        repo: String,
        rev: Option<String>,
        path: Option<String>,
    },
    /// A local directory package.
    Path { path: String },
}

/// A parsed `ptah package add` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddRequest {
    pub source: AddSource,
    /// `--as <alias>`; defaults per source form (package name's last
    /// segment, repository name, or directory name).
    pub alias: Option<String>,
}

/// What `add` recorded and whether it went on to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddOutcome {
    pub alias: String,
    pub entry: DependencyEntry,
    pub install: Option<InstallOutcome>,
}

/// `ptah package add`: resolve the newest matching version, record the
/// dependency under `[dependencies]` in the manifest (surgically, via
/// toml_edit), then — unless `install_after` is false (`--no-install`)
/// — run a full install.
pub async fn add(
    project: &PackageProject,
    client: &reqwest::Client,
    request: &AddRequest,
    install_after: bool,
) -> Result<AddOutcome, Error> {
    let manifest = manifest::deser_manifest(project).await?;
    manifest::require_luau_target(&manifest)?;

    // In-memory default-index injection happens in the registry arm
    // below: only that source form consults the default registry, so a
    // malformed override (PTAH_DEFAULT_INDEX) fails exactly the forms
    // that would use it.
    let (source, specifier, default_alias) = match &request.source {
        AddSource::Registry { name, version } => {
            let name = name
                .parse::<PackageName>()
                .map_err(|e| Error::InvalidSpec { source: e.to_string() })?;
            let version = match version {
                Some(v) => VersionReq::from_str(v)
                    .map_err(|e| Error::InvalidSpec { source: e.to_string() })?,
                None => VersionReq::STAR,
            };
            // In-memory default-index injection: ptah's own resolve
            // call sees the default registry without it ever reaching
            // the manifest file.
            let mut indices = manifest.clone();
            inject_default_index(&mut indices)?;
            let index_url = indices.indices.get(pesde::DEFAULT_INDEX_NAME).cloned();
            let index_url = match index_url {
                Some(url) => url,
                None => {
                    return Err(Error::ManifestEdit {
                        source: format!(
                            "no index named `{}` (add an [indices] entry or remove the empty one)",
                            pesde::DEFAULT_INDEX_NAME
                        ),
                    })
                }
            };
            let source = pesde::source::PackageSources::Pesde(
                pesde::source::pesde::PesdePackageSource::new(index_url),
            );
            let specifier = DependencySpecifiers::Pesde(PesdeDependencySpecifier {
                name,
                version,
                index: pesde::DEFAULT_INDEX_NAME.to_string(),
                target: None,
            });
            let alias = match &specifier {
                DependencySpecifiers::Pesde(spec) => spec.name.name().to_string(),
                _ => unreachable!(),
            };
            (source, specifier, alias)
        }
        AddSource::Git { repo, rev, path } => {
            let repo = gix_url(repo)?;
            let path = match path {
                Some(p) => Some(relative_path::RelativePathBuf::from(p.as_str())),
                None => None,
            };
            let source = pesde::source::PackageSources::Git(
                pesde::source::git::GitPackageSource::new(repo.clone()),
            );
            let specifier = DependencySpecifiers::Git(GitDependencySpecifier {
                repo,
                // An unpinned git dependency tracks the default branch
                // tip; installs stay pinned by the lockfile (reused by
                // specifier equality), exactly like a caret range.
                rev: rev.clone().unwrap_or_else(|| "HEAD".to_string()),
                path,
            });
            let alias = match &specifier {
                DependencySpecifiers::Git(spec) => repo_name(&spec.repo),
                _ => unreachable!(),
            };
            (source, specifier, alias)
        }
        AddSource::Path { path } => {
            // Record an absolute path: resolution must not depend on
            // the invocation directory.
            let abs = std::fs::canonicalize(path).map_err(|e| Error::InvalidSpec {
                source: format!("cannot resolve package path `{path}`: {e}"),
            })?;
            let source = pesde::source::PackageSources::Path(
                pesde::source::path::PathPackageSource,
            );
            let specifier = DependencySpecifiers::Path(PathDependencySpecifier { path: abs });
            let alias = match &specifier {
                DependencySpecifiers::Path(spec) => spec
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "package".to_string()),
                _ => unreachable!(),
            };
            (source, specifier, alias)
        }
    };

    let alias = match &request.alias {
        Some(alias) => alias.clone(),
        None => default_alias,
    };
    let alias = alias
        .parse::<Alias>()
        .map_err(|e| Error::InvalidSpec { source: e.to_string() })?;

    // Newest matching version. (Path sources ignore this — their
    // resolve result is the package itself.)
    let refreshed = RefreshedSources::new();
    refreshed
        .refresh(&source, &RefreshOptions { project: project.pesde().clone() })
        .await
        .map_err(|e| Error::Source {
            step: "refresh package source",
            source: error_chain(&e),
        })?;
    let (_, mut versions, _suggestions) = source
        .resolve(
            &specifier,
            &ResolveOptions {
                project: project.pesde().clone(),
                target: manifest.target.kind(),
                refreshed_sources: refreshed.clone(),
                loose_target: false,
            },
        )
        .await
        .map_err(|e| Error::Source {
            step: "resolve package",
            // Name the requested spec: pesde's own chain stops at the
            // source level without it (spec: the diagnostic names the
            // package spec).
            source: format!("`{specifier}`: {}", error_chain(&e)),
        })?;
    let Some((version_id, _)) = versions.pop_last() else {
        return Err(Error::Source {
            step: "resolve package",
            source: format!("no matching versions for `{specifier}`"),
        });
    };

    let entry = match &specifier {
        DependencySpecifiers::Pesde(spec) => DependencyEntry::Pesde {
            name: spec.name.to_string(),
            version: format!("^{}", version_id.version()),
            target: (version_id.target() != manifest.target.kind())
                .then(|| version_id.target().to_string()),
            index: None,
        },
        DependencySpecifiers::Git(spec) => DependencyEntry::Git {
            repo: spec.repo.to_bstring().to_string(),
            rev: spec.rev.clone(),
            path: spec.path.as_ref().map(|p| p.as_str().to_string()),
        },
        DependencySpecifiers::Path(spec) => DependencyEntry::Path {
            path: spec.path.display().to_string(),
        },
        DependencySpecifiers::Workspace(_) => {
            return Err(Error::InvalidSpec {
                source: "unsupported source form".to_string(),
            })
        }
    };

    // Surgical manifest edit.
    let text = manifest::read_manifest_text(project)?;
    let mut doc = manifest::parse_manifest_doc(&text)?;
    manifest::set_dependency(&mut doc, alias.as_str(), &entry)?;
    manifest::write_manifest_text(project, &doc.to_string())?;

    let install_outcome = if install_after {
        Some(install(project, client, &InstallOptions::default()).await?)
    } else {
        None
    };

    Ok(AddOutcome {
        alias: alias.to_string(),
        entry,
        install: install_outcome,
    })
}

/// What `remove` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveOutcome {
    pub alias: String,
    pub install: Option<InstallOutcome>,
}

/// `ptah package remove <alias>`: delete the dependency entry from the
/// manifest (any dependency table) and — unless `install_after` is
/// false — reinstall so the lockfile and `luau_packages/` follow.
pub async fn remove(
    project: &PackageProject,
    client: &reqwest::Client,
    alias: &str,
    install_after: bool,
) -> Result<RemoveOutcome, Error> {
    // A syntactically invalid alias can never be recorded: usage error
    // before touching the manifest.
    let alias = alias
        .parse::<Alias>()
        .map_err(|e| Error::InvalidSpec { source: e.to_string() })?;

    let text = manifest::read_manifest_text(project)?;
    let mut doc = manifest::parse_manifest_doc(&text)?;
    if !manifest::remove_dependency(&mut doc, alias.as_str()) {
        return Err(Error::UnknownAlias {
            alias: alias.to_string(),
        });
    }
    manifest::write_manifest_text(project, &doc.to_string())?;

    let install_outcome = if install_after {
        Some(install(project, client, &InstallOptions::default()).await?)
    } else {
        None
    };

    Ok(RemoveOutcome {
        alias: alias.to_string(),
        install: install_outcome,
    })
}

// ----------------------------------------------------------------------
// Internals
// ----------------------------------------------------------------------

/// Parse a git repository URL. Plain absolute or relative local paths
/// (no scheme) are canonicalized into explicit `file://` URLs — gix
/// would otherwise parse them as remote hostnames.
fn gix_url(repo: &str) -> Result<gix::Url, Error> {
    let candidate =
        if repo.starts_with('/') || repo.starts_with("./") || repo.starts_with("../") {
            let abs = std::fs::canonicalize(repo).map_err(|e| Error::InvalidSpec {
                source: format!("git repository `{repo}` not found: {e}"),
            })?;
            format!("file://{}", abs.display())
        } else {
            repo.to_string()
        };
    gix::Url::try_from(candidate).map_err(|e| Error::InvalidSpec {
        source: format!("invalid git repository `{repo}`: {e}"),
    })
}

/// Render an error's full source chain — pesde errors nest the real
/// cause (gix, reqwest, io) two or three levels deep, and the top
/// message alone ("error refreshing package source") says nothing.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut chain = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        chain.push_str(": ");
        chain.push_str(&s.to_string());
        source = s.source();
    }
    chain
}

/// The repository name of a git URL (last path segment, `.git`
/// stripped) — the default alias for git dependencies.
fn repo_name(url: &gix::Url) -> String {
    url.path
        .to_string()
        .rsplit('/')
        .next()
        .map(|s| s.trim_end_matches(".git").to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "package".to_string())
}

/// Inject the default index into an in-memory manifest when its
/// indices map is empty (the in-memory view ptah resolves against; the
/// manifest file never grows the table). Fails with a usage-class
/// error when the default index URL (env override or const) does not
/// parse — a value ptah cannot use, so retrying unchanged cannot
/// succeed.
fn inject_default_index(manifest: &mut Manifest) -> Result<(), Error> {
    if manifest.indices.is_empty() {
        let url = crate::parse_default_index_url(&crate::default_index_url())?;
        manifest
            .indices
            .insert(pesde::DEFAULT_INDEX_NAME.to_string(), url);
    }
    Ok(())
}

/// Run `f` with the default index materialized into the on-disk
/// manifest, restoring the original text afterwards. Pesde's library
/// resolution re-reads the manifest from disk (its `Manifest` type
/// does not round-trip `indices`), so the default registry is only
/// reachable from a manifest that carries the table — ptah supplies it
/// for the duration of the operation and puts the user's bytes back,
/// so the committed manifest never contains a ptah-written indices
/// table. A crash between materialize and restore can leave the table
/// behind — benign: it names the same URL ptah would inject, and the
/// next operation treats it as user configuration.
async fn with_default_index<T, F>(project: &PackageProject, f: F) -> Result<T, Error>
where
    F: AsyncFnOnce(&PackageProject) -> Result<T, Error>,
{
    let original = manifest::read_manifest_text(project)?;
    let mut doc = manifest::parse_manifest_doc(&original)?;
    let injected = manifest::materialize_default_index(&mut doc)?;
    if injected {
        manifest::write_manifest_text(project, &doc.to_string())?;
    }
    let result = f(project).await;
    if injected {
        manifest::write_manifest_text(project, &original)?;
    }
    result
}

/// Compare the manifest against the lockfile: fresh, missing, or stale
/// with the reason. Staleness is ptah's `--locked` contract: name,
/// target, overrides, and the direct dependency specification sets
/// must match in both directions (pesde's CLI only checks that every
/// manifest dependency appears in the lockfile).
pub async fn check_lockfile(
    project: &PackageProject,
    manifest: &Manifest,
) -> Result<LockfileCheck, Error> {
    let lockfile = match project.pesde().deser_lockfile().await {
        Ok(lockfile) => lockfile,
        Err(pesde::errors::LockfileReadError::Io(e))
            if e.kind() == std::io::ErrorKind::NotFound =>
        {
            return Ok(LockfileCheck::Missing);
        }
        Err(e) => {
            return Err(Error::Install {
                source: format!("cannot read pesde.lock: {e}"),
            })
        }
    };

    if manifest.name != lockfile.name {
        return Ok(LockfileCheck::Stale {
            reason: format!(
                "manifest names {}, lockfile names {}",
                manifest.name, lockfile.name
            ),
        });
    }
    if manifest.version != lockfile.version {
        return Ok(LockfileCheck::Stale {
            reason: format!(
                "manifest version {} != lockfile version {}",
                manifest.version, lockfile.version
            ),
        });
    }
    if manifest.target.kind() != lockfile.target {
        return Ok(LockfileCheck::Stale {
            reason: format!(
                "manifest target {} != lockfile target {}",
                manifest.target.kind(),
                lockfile.target
            ),
        });
    }
    if resolve_overrides(manifest)? != lockfile.overrides {
        return Ok(LockfileCheck::Stale {
            reason: "overrides changed".to_string(),
        });
    }

    let manifest_specs: std::collections::HashSet<_> = manifest
        .all_dependencies()
        .map_err(|e| Error::ManifestEdit {
            source: format!("duplicate dependency alias: {e}"),
        })?
        .into_iter()
        .map(|(alias, (spec, ty))| (alias, spec, ty))
        .collect();
    let lockfile_specs: std::collections::HashSet<_> = lockfile
        .graph
        .iter()
        .filter_map(|(_, node)| {
            node.direct
                .as_ref()
                .map(|(alias, spec, ty)| (alias.clone(), spec.clone(), *ty))
        })
        .collect();
    if manifest_specs != lockfile_specs {
        return Ok(LockfileCheck::Stale {
            reason: "dependency specifications changed".to_string(),
        });
    }

    Ok(LockfileCheck::Fresh(Box::new(lockfile)))
}

/// Resolve alias-referencing overrides to their specifiers (pesde's
/// CLI helper, over public types): an `OverrideSpecifier::Alias` is
/// replaced by the dependency registered under that alias.
fn resolve_overrides(
    manifest: &Manifest,
) -> Result<BTreeMap<pesde::manifest::overrides::OverrideKey, DependencySpecifiers>, Error> {
    let mut dependencies = None;
    let mut overrides = BTreeMap::new();
    for (key, spec) in &manifest.overrides {
        let resolved = match spec {
            OverrideSpecifier::Specifier(spec) => spec.clone(),
            OverrideSpecifier::Alias(alias) => {
                let deps = dependencies
                    .get_or_insert_with(|| {
                        manifest.all_dependencies().map_err(|e| Error::ManifestEdit {
                            source: format!("duplicate dependency alias: {e}"),
                        })
                    })
                    .as_ref()
                    .unwrap();
                deps.get(alias)
                    .map(|(spec, _)| spec.clone())
                    .ok_or_else(|| Error::ManifestEdit {
                        source: format!("override alias `{alias}` not found in manifest"),
                    })?
            }
        };
        overrides.insert(key.clone(), resolved);
    }
    Ok(overrides)
}

/// Serialize a lockfile to exactly the bytes pesde's own writer
/// produces (header, format line, body) — used for the
/// nothing-changed check so an up-to-date install rewrites no file.
fn lockfile_text(lockfile: &Lockfile) -> Result<String, Error> {
    let body = toml::to_string(lockfile).map_err(|e| Error::Io {
        context: "cannot serialize pesde.lock".to_string(),
        source: std::io::Error::other(e.to_string()),
    })?;
    Ok(format!(
        "# This file is automatically @generated by pesde.\n# It is not intended for manual editing.\nformat = {}\n{body}",
        pesde::lockfile::CURRENT_FORMAT
    ))
}

/// Path to a package's linked linker module for `luau` targets
/// (`<project>/.ptah/luau_packages/<alias>.luau`), relative to the
/// project root — the value ptah records in the root `.luaurc`. The
/// `.luaurc` lives at the project root, so the value is always this
/// fixed relative form.
pub fn linker_module_path(alias: &str) -> String {
    format!(
        "./.ptah/{}",
        std::path::Path::new(
            &pesde::manifest::target::TargetKind::Luau.packages_folder(
                pesde::manifest::target::TargetKind::Luau,
            )
        )
        .join(alias)
        .to_string_lossy()
        .replace('\\', "/")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_name_extracts_last_segment() {
        assert_eq!(
            repo_name(&gix_url("https://github.com/patextreme/ptah-libs").unwrap()),
            "ptah-libs"
        );
        assert_eq!(
            repo_name(&gix_url("https://github.com/x/y.git").unwrap()),
            "y"
        );
    }

    #[test]
    fn linker_module_path_points_into_luau_packages() {
        assert_eq!(linker_module_path("hello"), "./.ptah/luau_packages/hello");
    }

    #[test]
    fn lockfile_text_carries_the_pesde_header() {
        let lockfile = Lockfile {
            name: "abc/x".parse().unwrap(),
            version: "0.1.0".parse().unwrap(),
            target: pesde::manifest::target::TargetKind::Luau,
            overrides: BTreeMap::new(),
            graph: BTreeMap::new(),
            workspace: BTreeMap::new(),
        };
        let text = lockfile_text(&lockfile).unwrap();
        assert!(text.starts_with("# This file is automatically @generated by pesde.\n"));
        assert!(text.contains("format = 2"), "{text}");
    }

    // ------------------------------------------------------------------
    // Driver integration: a local path-source package in tempdirs —
    // fully offline, no registry, no git.
    // ------------------------------------------------------------------

    use std::path::PathBuf;

    fn fixture_project() -> (tempfile::TempDir, PackageProject) {
        let dir = tempfile::tempdir().unwrap();
        let ptah = dir.path().join(".ptah");
        std::fs::create_dir_all(&ptah).unwrap();
        std::fs::write(ptah.join("pesde.toml"), manifest::skeleton("proj")).unwrap();
        let project = PackageProject::open(dir.path());
        (dir, project)
    }

    fn fixture_package(name: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::fs::write(
            path.join("pesde.toml"),
            format!(
                r#"name = "{name}"
version = "0.1.0"

[target]
environment = "luau"
lib = "init.luau"
"#
            ),
        )
        .unwrap();
        std::fs::write(
            path.join("init.luau"),
            "--!strict\nreturn { greet = 'hello from package' }\n",
        )
        .unwrap();
        (dir, path)
    }

    fn client() -> reqwest::Client {
        reqwest::Client::new()
    }

    #[tokio::test]
    async fn add_from_a_path_source_installs_and_records() {
        let (_proj_dir, project) = fixture_project();
        let (_pkg_dir, pkg) = fixture_package("abc/hello");

        let outcome = add(
            &project,
            &client(),
            &AddRequest {
                source: AddSource::Path {
                    path: pkg.display().to_string(),
                },
                alias: Some("hello".into()),
            },
            true,
        )
        .await
        .unwrap();

        // The manifest records the absolute path under the given alias.
        assert_eq!(outcome.alias, "hello");
        assert_eq!(
            outcome.entry,
            DependencyEntry::Path {
                path: pkg.display().to_string()
            }
        );
        let manifest_text = std::fs::read_to_string(project.ptah_dir().join("pesde.toml")).unwrap();
        assert!(
            manifest_text.contains(&format!("path = \"{}\"", pkg.display())),
            "{manifest_text}"
        );
        // No [indices] table appeared in the manifest (the docs
        // comment may mention it; an actual table may not exist).
        let active_lines = manifest_text
            .lines()
            .map(str::trim)
            .filter(|l| !l.starts_with('#'))
            .collect::<Vec<_>>();
        assert!(
            !active_lines.iter().any(|l| l.contains("[indices]")),
            "indices table leaked into the manifest: {manifest_text}"
        );

        // The lockfile exists and the package is linked under
        // luau_packages/.pesde/ + a linker module for the alias.
        let ptah = project.ptah_dir();
        assert!(ptah.join("pesde.lock").is_file());
        let linker = ptah.join(format!("luau_packages/{}.luau", outcome.alias));
        assert!(linker.is_file(), "linker module missing: {}", linker.display());
        let container_root = ptah.join("luau_packages/.pesde");
        assert!(container_root.is_dir(), "container root missing");
        assert!(outcome.install.as_ref().unwrap().lockfile_written);
    }

    #[tokio::test]
    async fn add_auto_alias_from_an_invalid_default_is_a_usage_error() {
        // A tempdir basename carries a dot — not a valid alias; the
        // error names the alias problem (pesde's own behavior: use
        // --as).
        let (_proj_dir, project) = fixture_project();
        let (_pkg_dir, pkg) = fixture_package("abc/hello");
        let err = add(
            &project,
            &client(),
            &AddRequest {
                source: AddSource::Path {
                    path: pkg.display().to_string(),
                },
                alias: None,
            },
            true,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::InvalidSpec { .. }), "{err}");
        assert!(err.is_usage());
        assert!(err.to_string().contains("alias"), "{err}");
    }

    #[tokio::test]
    async fn install_is_idempotent_when_up_to_date() {
        let (_proj_dir, project) = fixture_project();
        let (_pkg_dir, pkg) = fixture_package("abc/hello");
        add(
            &project,
            &client(),
            &AddRequest {
                source: AddSource::Path {
                    path: pkg.display().to_string(),
                },
                alias: Some("hello".into()),
            },
            true,
        )
        .await
        .unwrap();

        // Second install: same graph, lockfile bytes unchanged.
        let lock_path = project.ptah_dir().join("pesde.lock");
        let before = std::fs::read(&lock_path).unwrap();
        let before_linker =
            std::fs::read(project.ptah_dir().join("luau_packages/hello.luau")).unwrap();
        let outcome = install(&project, &client(), &InstallOptions::default())
            .await
            .unwrap();
        assert!(!outcome.lockfile_written, "nothing should change");
        assert_eq!(std::fs::read(&lock_path).unwrap(), before);
        assert_eq!(
            std::fs::read(project.ptah_dir().join("luau_packages/hello.luau")).unwrap(),
            before_linker
        );
    }

    #[tokio::test]
    async fn remove_uninstalls_the_package() {
        let (_proj_dir, project) = fixture_project();
        let (_pkg_dir, pkg) = fixture_package("abc/hello");
        add(
            &project,
            &client(),
            &AddRequest {
                source: AddSource::Path {
                    path: pkg.display().to_string(),
                },
                alias: Some("hello".into()),
            },
            true,
        )
        .await
        .unwrap();

        let outcome = remove(&project, &client(), "hello", true).await.unwrap();
        assert_eq!(outcome.alias, "hello");
        let manifest_text = std::fs::read_to_string(project.ptah_dir().join("pesde.toml")).unwrap();
        assert!(!manifest_text.contains("hello"), "{manifest_text}");
        // The graph dropped to zero packages; the lockfile reflects
        // it and the linker module no longer resolves.
        assert_eq!(outcome.install.unwrap().packages, 0);
        assert!(
            !project.ptah_dir().join("luau_packages/hello.luau").exists(),
            "removed package's linker module must be gone"
        );
    }

    // ------------------------------------------------------------------
    // Locked-mode staleness (task 2.3)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn locked_install_requires_a_lockfile() {
        let (_proj_dir, project) = fixture_project();
        let err = install(
            &project,
            &client(),
            &InstallOptions {
                locked: true,
                fresh: false,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::LockfileMissing { .. }), "{err}");
        assert!(!err.is_usage());
        assert!(err.to_string().contains("lockfile is missing"), "{err}");
    }

    #[tokio::test]
    async fn locked_install_rejects_a_stale_lockfile() {
        let (_proj_dir, project) = fixture_project();
        let (_pkg_dir, pkg) = fixture_package("abc/hello");
        add(
            &project,
            &client(),
            &AddRequest {
                source: AddSource::Path {
                    path: pkg.display().to_string(),
                },
                alias: Some("hello".into()),
            },
            true,
        )
        .await
        .unwrap();

        // Hand-edit the manifest: a new dependency without
        // reinstalling → the lockfile is stale.
        let manifest_path = project.ptah_dir().join("pesde.toml");
        let text = std::fs::read_to_string(&manifest_path).unwrap();
        let stale = text.replace(
            "[target]",
            "[dependencies.extra]\nname = \"abc/extra\"\nversion = \"^0.1.0\"\n\n[target]",
        );
        assert_ne!(stale, text);
        std::fs::write(&manifest_path, stale).unwrap();

        let err = install(
            &project,
            &client(),
            &InstallOptions {
                locked: true,
                fresh: false,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::LockfileStale { .. }), "{err}");
        assert!(err.to_string().contains("out of sync"), "{}", err);
    }

    #[tokio::test]
    async fn locked_install_succeeds_on_a_fresh_lockfile() {
        let (_proj_dir, project) = fixture_project();
        let (_pkg_dir, pkg) = fixture_package("abc/hello");
        add(
            &project,
            &client(),
            &AddRequest {
                source: AddSource::Path {
                    path: pkg.display().to_string(),
                },
                alias: Some("hello".into()),
            },
            true,
        )
        .await
        .unwrap();

        let lock_path = project.ptah_dir().join("pesde.lock");
        let before = std::fs::read(&lock_path).unwrap();
        // A "fresh clone": packages and caches wiped, lockfile kept
        // (path packages never touch the cache, so its absence is
        // fine).
        let _ = std::fs::remove_dir_all(project.ptah_dir().join("luau_packages"));
        let _ = std::fs::remove_dir_all(project.ptah_dir().join(".pesde"));

        let outcome = install(
            &project,
            &client(),
            &InstallOptions {
                locked: true,
                fresh: false,
            },
        )
        .await
        .unwrap();
        assert_eq!(outcome.packages, 1);
        assert!(!outcome.lockfile_written, "locked install never rewrites");
        assert_eq!(std::fs::read(&lock_path).unwrap(), before);
        assert!(project.ptah_dir().join("luau_packages/hello.luau").is_file());
    }

    #[tokio::test]
    async fn remove_unknown_alias_is_a_usage_error() {
        let (_proj_dir, project) = fixture_project();
        let err = remove(&project, &client(), "nope", true).await.unwrap_err();
        assert!(matches!(err, Error::UnknownAlias { .. }), "{err}");
        assert!(err.is_usage());
    }

    // ------------------------------------------------------------------
    // Git sources (task 2.4): a local fixture repository, cloned
    // in-process by pesde's gix transport — no network.
    // ------------------------------------------------------------------

    fn fixture_git_package() -> (tempfile::TempDir, crate::fixtures::GitFixture) {
        let dir = tempfile::tempdir().unwrap();
        let fixture = crate::fixtures::git_repo(
            dir.path(),
            &[
                crate::fixtures::FileSpec {
                    path: "pesde.toml",
                    contents: r#"name = "abc/hello"
version = "0.1.0"

[target]
environment = "luau"
lib = "init.luau"
"#,
                },
                crate::fixtures::FileSpec {
                    path: "init.luau",
                    contents: "--!strict\nreturn { greet = 'hi from git' }\n",
                },
            ],
        );
        (dir, fixture)
    }

    #[tokio::test]
    async fn add_from_a_git_source_records_and_pins() {
        let (_proj_dir, project) = fixture_project();
        let (_repo_dir, repo) = fixture_git_package();

        let outcome = add(
            &project,
            &client(),
            &AddRequest {
                source: AddSource::Git {
                    repo: repo.dir.display().to_string(),
                    rev: None,
                    path: None,
                },
                alias: Some("hello".into()),
            },
            true,
        )
        .await
        .unwrap();

        // Recorded spec: the repository and the defaulted rev.
        assert_eq!(
            outcome.entry,
            DependencyEntry::Git {
                repo: format!("file://{}", repo.dir.display()),
                rev: "HEAD".to_string(),
                path: None,
            }
        );

        // Lockfile pins the resolved commit's tree: the package's
        // version id embeds the tree id, and the git pkg_ref records
        // it explicitly.
        let lock = std::fs::read_to_string(project.ptah_dir().join("pesde.lock")).unwrap();
        assert!(lock.contains("0.0.0-"), "git version pin: {lock}");
        assert!(lock.contains("tree_id"), "tree pinned: {lock}");

        // Installed: linker module for the alias exists.
        assert!(project
            .ptah_dir()
            .join("luau_packages/hello.luau")
            .is_file());

        // A locked reinstall from a "fresh clone" restores exactly the
        // same pin (HEAD did not move).
        let _ = std::fs::remove_dir_all(project.ptah_dir().join("luau_packages"));
        let _ = std::fs::remove_dir_all(project.ptah_dir().join(".pesde"));
        let lock_before = std::fs::read(project.ptah_dir().join("pesde.lock")).unwrap();
        let outcome = install(
            &project,
            &client(),
            &InstallOptions {
                locked: true,
                fresh: false,
            },
        )
        .await
        .unwrap();
        assert!(!outcome.lockfile_written);
        assert_eq!(
            std::fs::read(project.ptah_dir().join("pesde.lock")).unwrap(),
            lock_before
        );
        assert!(project
            .ptah_dir()
            .join("luau_packages/hello.luau")
            .is_file());
    }

    #[tokio::test]
    async fn add_from_a_git_subdirectory_records_the_path() {
        let (_proj_dir, project) = fixture_project();
        let (_repo_dir, _repo) = fixture_git_package();

        // The same fixture, but the package "lives" in a subdirectory:
        // rebuild the repo with the files nested under pkg/hello.
        let dir = tempfile::tempdir().unwrap();
        let repo = crate::fixtures::git_repo(
            dir.path(),
            &[
                crate::fixtures::FileSpec {
                    path: "pkg/hello/pesde.toml",
                    contents: r#"name = "abc/hello"
version = "0.1.0"

[target]
environment = "luau"
lib = "init.luau"
"#,
                },
                crate::fixtures::FileSpec {
                    path: "pkg/hello/init.luau",
                    contents: "--!strict\nreturn { greet = 'hi from subdir' }\n",
                },
            ],
        );

        let outcome = add(
            &project,
            &client(),
            &AddRequest {
                source: AddSource::Git {
                    repo: repo.dir.display().to_string(),
                    rev: None,
                    path: Some("pkg/hello".into()),
                },
                alias: Some("hello".into()),
            },
            true,
        )
        .await
        .unwrap();
        assert!(
            matches!(&outcome.entry, DependencyEntry::Git { path: Some(p), .. } if p == "pkg/hello"),
            "subdirectory recorded: {:?}",
            outcome.entry
        );
        assert!(project
            .ptah_dir()
            .join("luau_packages/hello.luau")
            .is_file());
    }
}
