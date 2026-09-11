//! `ptah package` — the package-management command group. Thin
//! composition over the `ptah-pesde` adapter: argument shapes and
//! exit-code contract here, Pesde semantics there. Every command
//! discovers the project from the invocation directory, runs on its
//! own tokio runtime (like `run`), prints ptah-style stage lines, and
//! synchronizes the root `.luaurc` after any manifest change.

use std::path::PathBuf;
use std::process::ExitCode;

use ptah_pesde::driver::{AddRequest, AddSource, InstallOptions};
use ptah_pesde::{luaurc, manifest, Error, PackageProject};

/// The commit/ignore guidance printed by the first successful
/// mutating package command in a project (one that writes the first
/// lockfile). ptah prints hints, never edits VCS config.
const GUIDANCE: &str = "Source control: commit .ptah/pesde.toml, .ptah/pesde.lock, and the \
     root .luaurc; ignore .ptah/luau_packages/ and .ptah/.pesde/.";

/// The parsed `ptah package add` arguments (validated shapes only —
/// the source-form mutual exclusions are clap's).
#[derive(Debug, Clone)]
pub struct AddArgs {
    pub package: Option<String>,
    pub git: Option<String>,
    pub rev: Option<String>,
    pub path: Option<String>,
    pub alias: Option<String>,
    pub no_install: bool,
}

/// `ptah package add`.
pub fn add(args: AddArgs) -> ExitCode {
    execute(|project, client| {
        let source = match (&args.package, &args.git, &args.path) {
            (Some(spec), None, None) => {
                let (name, version) = match spec.split_once('@') {
                    Some((name, version)) => (name.to_string(), Some(version.to_string())),
                    None => (spec.clone(), None),
                };
                AddSource::Registry { name, version }
            }
            (None, Some(repo), path) => AddSource::Git {
                repo: repo.clone(),
                rev: args.rev.clone(),
                path: path.clone(),
            },
            (None, None, Some(path)) => AddSource::Path {
                path: path.clone(),
            },
            // clap's conflicts encode the mutual exclusions; the
            // remaining combinations are unreachable.
            _ => unreachable!("clap validated the source forms"),
        };
        async move {
            let outcome = ptah_pesde::driver::add(
                &project,
                &client,
                &AddRequest {
                    source,
                    alias: args.alias.clone(),
                },
                !args.no_install,
            )
            .await?;
            println!(
                "added `{}` to .ptah/pesde.toml as dependency `{}`",
                outcome.entry.summary(),
                outcome.alias
            );
            report_install(outcome.install.as_ref());
            Ok(outcome.install.is_some())
        }
    })
}

/// `ptah package remove <alias>`.
pub fn remove(alias: String, no_install: bool) -> ExitCode {
    execute(|project, client| {
        let alias = alias.clone();
        async move {
            let outcome = ptah_pesde::driver::remove(&project, &client, &alias, !no_install)
                .await?;
            println!("removed dependency `{}` from .ptah/pesde.toml", outcome.alias);
            report_install(outcome.install.as_ref());
            Ok(outcome.install.is_some())
        }
    })
}

/// `ptah package install [--locked]`.
pub fn install(locked: bool) -> ExitCode {
    execute(|project, client| async move {
        let outcome = ptah_pesde::driver::install(
            &project,
            &client,
            &InstallOptions {
                locked,
                fresh: false,
            },
        )
        .await?;
        report_install(Some(&outcome));
        Ok(true)
    })
}

/// `ptah package update`.
pub fn update() -> ExitCode {
    execute(|project, client| async move {
        let outcome = ptah_pesde::driver::update(&project, &client).await?;
        report_install(Some(&outcome));
        Ok(true)
    })
}

/// Run one package command: discover the project (usage error naming
/// the search when none is found), execute on a fresh tokio runtime
/// with its own reqwest client, then perform the shared post-success
/// steps (`.luaurc` sync, first-run guidance) and map errors onto the
/// exit-code contract — usage failures 2, operational failures 1.
fn execute<F, Fut>(command: F) -> ExitCode
where
    F: FnOnce(PackageProject, reqwest::Client) -> Fut,
    Fut: std::future::Future<Output = Result<bool, Error>>,
{
    let invocation_dir =
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let project = match PackageProject::discover(&invocation_dir) {
        Ok(project) => project,
        Err(e) => return report_error(&e),
    };
    // "First mutating command" detection: the project has no lockfile
    // yet (the first full package operation writes it).
    let first = !project.ptah_dir().join("pesde.lock").exists();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let result = runtime.block_on(command(project.clone(), reqwest::Client::new()))
        .and_then(|ran_install| {
            sync_luaurc(&project).map(|()| ran_install)
        });
    runtime.shutdown_background();

    match result {
        Ok(ran_install) => {
            if first && ran_install {
                println!("{GUIDANCE}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => report_error(&e),
    }
}

/// Print one line per completed stage (design D5's reporter shape).
fn report_install(outcome: Option<&ptah_pesde::driver::InstallOutcome>) {
    let Some(outcome) = outcome else {
        return;
    };
    println!("resolved {} package(s)", outcome.packages);
    println!("installed into .ptah/luau_packages/");
    if outcome.lockfile_written {
        println!("wrote .ptah/pesde.lock");
    }
}

/// Synchronize the root `.luaurc` from the manifest's dependency
/// tables and report what changed (collisions are diagnostics; the
/// user's entry wins — the package's `@alias` require fails loudly).
fn sync_luaurc(project: &PackageProject) -> Result<(), Error> {
    let text = manifest::read_manifest_text(project)?;
    let aliases = manifest::dependency_aliases(&text)?;
    let report = luaurc::sync(project, &aliases)?;
    if report.created {
        println!("created .luaurc (package aliases)");
    } else if report.updated {
        println!("updated .luaurc (package aliases)");
    }
    for alias in &report.collisions {
        eprintln!(
            "warning: alias `{alias}` in .luaurc is user-owned; the package dependency \
             `{alias}` will not be requirable as @{alias} until it is renamed or removed"
        );
    }
    Ok(())
}

/// ptah-style error reporting with the package exit-code contract:
/// usage-class failures (bad spec, no project, unparseable manifest)
/// exit 2; operational failures exit 1.
fn report_error(e: &Error) -> ExitCode {
    eprintln!("error: {e}");
    ExitCode::from(if e.is_usage() { 2 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guidance_names_every_file_and_directory() {
        for name in [
            "pesde.toml",
            "pesde.lock",
            ".luaurc",
            "luau_packages/",
            ".pesde/",
        ] {
            assert!(GUIDANCE.contains(name), "guidance must name {name}: {GUIDANCE}");
        }
    }
}
