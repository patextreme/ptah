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
    normalize_git_cache(project);

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
    normalize_git_cache(project);

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

/// The fetch refspec that makes `git fetch` advance a cached bare
/// repository's local branch refs, not just its remote-tracking refs.
const LOCAL_HEADS_REFSPEC: &str = "+refs/heads/*:refs/heads/*";

/// Normalize every cached git dependency repository so that Pesde's own
/// fetch advances local branch refs, not only the remote-tracking refs.
///
/// Pesde 0.7.4 refreshes its git caches with the clone's default refspec
/// (`+refs/heads/*:refs/remotes/origin/*`), so after the initial clone
/// `refs/heads/<branch>` is frozen while `refs/remotes/origin/<branch>`
/// advances. Pesde's `resolve` then rev-parses `HEAD`/`<branch>` against
/// the frozen local ref, so unpinned dependencies never advance and
/// `--rev <branch>` reads a stale tree. Appending [`LOCAL_HEADS_REFSPEC`]
/// to the cached default remote makes Pesde's unchanged fetch update the
/// local refs too. The coupling to Pesde's private cache layout
/// (`<data_dir>/git_repos/<hash>`) is deliberate — the engine is pinned
/// exactly and the regression tests exercise this real path.
///
/// Best-effort and idempotent: a missing cache directory short-circuits,
/// unreadable or non-repository entries are skipped, and a repository
/// whose default remote already carries the mapping is left untouched
/// (no config churn).
fn normalize_git_cache(project: &PackageProject) {
    let dir = project.pesde().data_dir().join("git_repos");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let _ = normalize_cached_repo(&entry.path());
    }
}

/// Append [`LOCAL_HEADS_REFSPEC`] to one cached bare repository's default
/// remote when the mapping is absent, preserving the existing refspecs by
/// appending rather than replacing them.
fn normalize_cached_repo(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let repo = gix::open(path)?;
    let Some(remote) = repo.find_default_remote(gix::remote::Direction::Fetch) else {
        return Ok(());
    };
    let remote = remote?;
    if remote
        .refspecs(gix::remote::Direction::Fetch)
        .iter()
        .any(|spec| spec.to_ref().to_bstring() == LOCAL_HEADS_REFSPEC)
    {
        return Ok(());
    }

    let remote = remote.with_refspecs([LOCAL_HEADS_REFSPEC], gix::remote::Direction::Fetch)?;
    let config_path = repo.path().join("config");
    let mut config =
        gix::config::File::from_path_no_includes(config_path.clone(), gix::config::Source::Local)?;
    remote.save_to(&mut config)?;
    let mut file = std::fs::File::create(&config_path)?;
    config.write_to(&mut file)?;
    Ok(())
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
/// replaced by the dependency registered under that alias. A manifest
/// whose dependency tables share an alias fails `all_dependencies`
/// here — the failure is propagated, never unwrapped: this runs inside
/// `check_lockfile` before any other all-dependencies consumer, so a
/// panic here is the user-facing failure (exit 101, against the
/// 0/1/2 exit-code contract) instead of the intended usage error.
fn resolve_overrides(
    manifest: &Manifest,
) -> Result<BTreeMap<pesde::manifest::overrides::OverrideKey, DependencySpecifiers>, Error> {
    // Lazily computed once: alias -> (specifier, type), the map the
    // alias-form overrides resolve against.
    let mut dependencies = None;
    let mut overrides = BTreeMap::new();
    for (key, spec) in &manifest.overrides {
        let resolved = match spec {
            OverrideSpecifier::Specifier(spec) => spec.clone(),
            OverrideSpecifier::Alias(alias) => {
                if dependencies.is_none() {
                    dependencies =
                        Some(manifest.all_dependencies().map_err(|e| Error::ManifestEdit {
                            source: format!("duplicate dependency alias: {e}"),
                        })?);
                }
                // Infallible: populated immediately above.
                let deps = dependencies.as_ref().expect("populated above");
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

    #[tokio::test]
    async fn a_duplicate_alias_with_an_alias_form_override_is_a_usage_error() {
        // Regression: resolve_overrides unwrapped the very error it
        // mapped. With a lockfile whose name/version/target match,
        // check_lockfile reaches the overrides comparison before any
        // other all-dependencies consumer, so every install-shaped
        // command panicked (exit 101) instead of the usage error.
        let dir = tempfile::tempdir().unwrap();
        let ptah = dir.path().join(".ptah");
        std::fs::create_dir_all(&ptah).unwrap();
        // The manifest parses (each dependency table deserializes
        // independently; only all_dependencies sees the conflict), and
        // the plain-string override value deserializes as the untagged
        // Alias variant, which consults the dependency map.
        std::fs::write(
            ptah.join("pesde.toml"),
            r#"
name = "abc/x"
version = "0.1.0"

[target]
environment = "luau"

[dependencies]
hello = { name = "abc/hello", version = "^0.1.0" }

[dev_dependencies]
hello = { name = "abc/hello", version = "^0.2.0" }

[overrides]
"lib>hello" = "hello"
"#,
        )
        .unwrap();
        // A matching lockfile (graph/overrides default empty): the
        // overrides comparison runs before the dependency-spec check.
        std::fs::write(
            ptah.join("pesde.lock"),
            "# This file is automatically @generated by pesde.\n# It is not intended for manual editing.\nformat = 2\nname = \"abc/x\"\nversion = \"0.1.0\"\ntarget = \"luau\"\n",
        )
        .unwrap();
        let project = PackageProject::open(dir.path());
        for locked in [false, true] {
            let err = install(
                &project,
                &client(),
                &InstallOptions {
                    locked,
                    fresh: false,
                },
            )
            .await
            .unwrap_err();
            assert!(matches!(err, Error::ManifestEdit { .. }), "{err}");
            assert!(err.is_usage(), "exit 2, not a crash: {err}");
            assert!(err.to_string().contains("duplicate dependency alias"), "{err}");
        }
    }

    // ------------------------------------------------------------------
    // Git sources (task 2.4): a local fixture repository, cloned
    // in-process by pesde's gix transport — no network.
    // ------------------------------------------------------------------

    /// The git fixture package's manifest — shared by the initial fixture
    /// and the branch-advance steps so a new tip changes only the code.
    const GIT_PACKAGE_MANIFEST: &str = r#"name = "abc/hello"
version = "0.1.0"

[target]
environment = "luau"
lib = "init.luau"
"#;

    /// Build (or advance) the git fixture package's branch with a tip
    /// whose `init.luau` carries `marker`; the manifest is unchanged, so
    /// advancing changes only the code and produces a new tree.
    fn advance_git_package(dir: &std::path::Path, marker: &str) -> crate::fixtures::GitFixture {
        let contents = format!("--!strict\nreturn {{ greet = '{marker}' }}\n");
        crate::fixtures::git_repo(
            dir,
            &[
                crate::fixtures::FileSpec {
                    path: "pesde.toml",
                    contents: GIT_PACKAGE_MANIFEST,
                },
                crate::fixtures::FileSpec {
                    path: "init.luau",
                    contents: &contents,
                },
            ],
        )
    }

    fn fixture_git_package() -> (tempfile::TempDir, crate::fixtures::GitFixture) {
        let dir = tempfile::tempdir().unwrap();
        let fixture = advance_git_package(dir.path(), "hi from git");
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

    // ------------------------------------------------------------------
    // Stale git cache (change: fix-stale-git-cache)
    // ------------------------------------------------------------------

    /// Clone a fixture repository into the project's `git_repos` cache
    /// exactly as Pesde's refresh does (bare clone), so the helper sees
    /// the real cached layout: a default remote whose only fetch
    /// refspec is the remote-tracking one.
    fn fixture_cached_git_repo(project: &PackageProject) {
        let dir = tempfile::tempdir().unwrap();
        let fixture = crate::fixtures::git_repo(
            dir.path(),
            &[crate::fixtures::FileSpec {
                path: "init.luau",
                contents: "--!strict\nreturn { greet = 'cached' }\n",
            }],
        );
        let cache = project.pesde().data_dir().join("git_repos");
        std::fs::create_dir_all(&cache).unwrap();
        let url = gix::Url::try_from(format!("file://{}", fixture.dir.display())).unwrap();
        gix::prepare_clone_bare(url, cache.join("abc"))
            .unwrap()
            .fetch_only(gix::progress::Discard, &false.into())
            .unwrap();
    }

    fn cached_refspecs(project: &PackageProject) -> Vec<String> {
        let repo = gix::open(project.pesde().data_dir().join("git_repos/abc")).unwrap();
        let remote = repo
            .find_default_remote(gix::remote::Direction::Fetch)
            .expect("default remote")
            .unwrap();
        remote
            .refspecs(gix::remote::Direction::Fetch)
            .iter()
            .map(|spec| spec.to_ref().to_bstring().to_string())
            .collect()
    }

    /// Read every `init.luau` under an installed container, to compare
    /// installed content against a commit without knowing the exact
    /// versioned path Pesde generates.
    fn installed_init_contents(container: &std::path::Path) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![container.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.file_name().is_some_and(|n| n == "init.luau") {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        out.push(text);
                    }
                }
            }
        }
        out
    }

    #[tokio::test]
    async fn normalize_git_cache_adds_the_local_heads_refspec_once() {
        let (_proj_dir, project) = fixture_project();
        fixture_cached_git_repo(&project);
        let config_path = project.pesde().data_dir().join("git_repos/abc/config");

        let before = cached_refspecs(&project);
        assert!(
            !before.iter().any(|s| s == LOCAL_HEADS_REFSPEC),
            "fixture clone starts without the mapping: {before:?}"
        );

        normalize_git_cache(&project);
        let after = cached_refspecs(&project);
        assert!(
            after.iter().any(|s| s == LOCAL_HEADS_REFSPEC),
            "mapping added on the first call: {after:?}"
        );

        // Idempotent: the second pass rewrites nothing (byte-identical
        // config, same in-memory refspecs).
        let first_pass = std::fs::read(&config_path).unwrap();
        normalize_git_cache(&project);
        assert_eq!(
            std::fs::read(&config_path).unwrap(),
            first_pass,
            "a second call leaves the config unchanged"
        );
        assert_eq!(cached_refspecs(&project), after);
    }

    #[tokio::test]
    async fn normalize_git_cache_keeps_both_refspecs() {
        let (_proj_dir, project) = fixture_project();
        fixture_cached_git_repo(&project);
        normalize_git_cache(&project);
        let specs = cached_refspecs(&project);
        // Normalization appends; it must not drop the clone's original
        // remote-tracking mapping. gix returns the specs sorted, so the
        // order itself is not asserted (the local-heads mapping sorts
        // first); presence as a set is what matters.
        assert!(
            specs
                .iter()
                .any(|s| s == "+refs/heads/*:refs/remotes/origin/*"),
            "original remote-tracking refspec retained: {specs:?}"
        );
        assert!(
            specs.iter().any(|s| s == LOCAL_HEADS_REFSPEC),
            "local-heads refspec added: {specs:?}"
        );
    }

    #[tokio::test]
    async fn update_advances_an_unpinned_git_dependency() {
        let (_proj_dir, project) = fixture_project();
        let (_repo_dir, repo) = fixture_git_package();

        // Add unpinned: the manifest records `HEAD`, the cache clones at
        // the current tip, and the lockfile pins that commit.
        add(
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
        let lock_path = project.ptah_dir().join("pesde.lock");
        let initial_lock = std::fs::read_to_string(&lock_path).unwrap();
        assert!(
            initial_lock.contains(&repo.tree_id.to_string()),
            "initial pin is the first tip: {initial_lock}"
        );

        // The branch advances to a new tip with changed contents.
        let advanced = advance_git_package(&repo.dir, "new");
        assert_ne!(advanced.tree_id, repo.tree_id);

        update(&project, &client()).await.unwrap();

        let updated_lock = std::fs::read_to_string(&lock_path).unwrap();
        assert!(
            updated_lock.contains(&advanced.tree_id.to_string()),
            "update advanced the pin: {updated_lock}"
        );
        assert!(
            !updated_lock.contains(&repo.tree_id.to_string()),
            "the old tip is gone: {updated_lock}"
        );

        let container = project.ptah_dir().join("luau_packages/.pesde/abc+hello");
        let installed = installed_init_contents(&container);
        assert!(!installed.is_empty(), "installed under {container:?}");
        assert!(
            installed.iter().any(|s| s.contains("'new'")),
            "installed files match the new tip: {installed:?}"
        );
        assert!(
            !installed.iter().any(|s| s.contains("hi from git")),
            "no stale content survives: {installed:?}"
        );
    }

    #[tokio::test]
    async fn add_branch_rev_resolves_against_a_warm_cache() {
        let (_proj_dir, project) = fixture_project();
        let dir = tempfile::tempdir().unwrap();
        // The branch's first tip has no manifest yet.
        let initial = crate::fixtures::git_repo(
            dir.path(),
            &[crate::fixtures::FileSpec {
                path: "init.luau",
                contents: "--!strict\nreturn {}\n",
            }],
        );
        let repo = initial.dir.display().to_string();

        // First add: Pesde clones the cache during refresh, then fails
        // to resolve — the cached tip has no manifest. The cache stays.
        let err = add(
            &project,
            &client(),
            &AddRequest {
                source: AddSource::Git {
                    repo: repo.clone(),
                    rev: Some("main".into()),
                    path: None,
                },
                alias: Some("hello".into()),
            },
            true,
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("no manifest found"),
            "first add fails on the manifest-less tip: {err}"
        );

        // The branch advances to add the manifest and package code.
        let advanced = advance_git_package(dir.path(), "new");

        // Second add against the warm cache resolves at the new tip.
        let outcome = add(
            &project,
            &client(),
            &AddRequest {
                source: AddSource::Git {
                    repo,
                    rev: Some("main".into()),
                    path: None,
                },
                alias: Some("hello".into()),
            },
            true,
        )
        .await
        .unwrap();
        assert!(
            matches!(&outcome.entry, DependencyEntry::Git { rev, .. } if rev == "main"),
            "branch rev recorded: {:?}",
            outcome.entry
        );

        let lock = std::fs::read_to_string(project.ptah_dir().join("pesde.lock")).unwrap();
        assert!(
            lock.contains(&advanced.tree_id.to_string()),
            "resolved at the advanced tip: {lock}"
        );
        let container = project.ptah_dir().join("luau_packages/.pesde/abc+hello");
        assert!(
            installed_init_contents(&container)
                .iter()
                .any(|s| s.contains("'new'")),
            "installed files match the new tip"
        );
    }

    #[tokio::test]
    async fn install_keeps_a_git_branch_dependency_pinned() {
        let (_proj_dir, project) = fixture_project();
        let (_repo_dir, repo) = fixture_git_package();

        add(
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
        let lock_path = project.ptah_dir().join("pesde.lock");
        let pinned = std::fs::read_to_string(&lock_path).unwrap();
        assert!(pinned.contains(&repo.tree_id.to_string()));

        // The branch advances, but a plain install reuses the lockfile's
        // pin — only `update` moves a branch-tracking dependency.
        let advanced = advance_git_package(&repo.dir, "new");
        assert_ne!(advanced.tree_id, repo.tree_id);

        let outcome = install(&project, &client(), &InstallOptions::default())
            .await
            .unwrap();
        assert!(!outcome.lockfile_written, "install must not move the pin");

        let after = std::fs::read_to_string(&lock_path).unwrap();
        assert!(
            after.contains(&repo.tree_id.to_string()),
            "still pinned at the earlier commit: {after}"
        );
        assert!(
            !after.contains(&advanced.tree_id.to_string()),
            "install did not advance: {after}"
        );
        let installed =
            installed_init_contents(&project.ptah_dir().join("luau_packages/.pesde/abc+hello"));
        assert!(
            installed.iter().any(|s| s.contains("hi from git")),
            "installed files match the pinned commit: {installed:?}"
        );
        assert!(
            !installed.iter().any(|s| s.contains("'new'")),
            "no advanced content installed: {installed:?}"
        );
    }

    #[tokio::test]
    async fn update_keeps_a_commit_pinned_git_dependency() {
        let (_proj_dir, project) = fixture_project();
        let (_repo_dir, repo) = fixture_git_package();

        // Pin to the explicit commit SHA of the first tip.
        add(
            &project,
            &client(),
            &AddRequest {
                source: AddSource::Git {
                    repo: repo.dir.display().to_string(),
                    rev: Some(repo.head.to_string()),
                    path: None,
                },
                alias: Some("hello".into()),
            },
            true,
        )
        .await
        .unwrap();
        let lock_path = project.ptah_dir().join("pesde.lock");
        let pinned = std::fs::read_to_string(&lock_path).unwrap();
        assert!(pinned.contains(&repo.tree_id.to_string()));

        // The branch advances, but a commit-pinned dependency stays put.
        let advanced = advance_git_package(&repo.dir, "new");
        assert_ne!(advanced.tree_id, repo.tree_id);

        update(&project, &client()).await.unwrap();

        let after = std::fs::read_to_string(&lock_path).unwrap();
        assert!(
            after.contains(&repo.tree_id.to_string()),
            "commit pin preserved: {after}"
        );
        assert!(
            !after.contains(&advanced.tree_id.to_string()),
            "update did not move a commit-pinned dep: {after}"
        );
        let installed =
            installed_init_contents(&project.ptah_dir().join("luau_packages/.pesde/abc+hello"));
        assert!(
            installed.iter().any(|s| s.contains("hi from git")),
            "installed files match the pinned commit: {installed:?}"
        );
    }
}
