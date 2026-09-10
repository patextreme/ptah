//! Command-line interface: `ptah run <script.luau>` with output flags.

use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr as _;
use std::sync::Arc;

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

use crate::ask::{StdinAskProvider, stdin_is_tty, stdout_is_tty};
use crate::render::{RenderOptions, Renderer};
use crate::script::{self, RunConfig};
use ptah_core::config::AskProviderKind;
use ptah_core::ports::{ConfigSource, InteractionMode};

/// clap value parser over [`AskProviderKind`] — unknown values are
/// usage errors (exit 2) naming the accepted set (the same string
/// `PTAH_ASK` and `[ask] provider` parse through).
fn parse_ask_provider(value: &str) -> Result<AskProviderKind, String> {
    AskProviderKind::from_str(value)
}

#[derive(Parser, Debug)]
#[command(
    name = "ptah",
    version,
    about = "Luau-scripted multi-agent orchestration over the Agent Client Protocol"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run an orchestration script.
    Run {
        /// Path to the entry Luau script.
        script: PathBuf,
        /// Suppress all streaming render and diagnostics (ask prompts
        /// still render: they are required interaction, not noise).
        #[arg(long)]
        quiet: bool,
        /// Show runtime lifecycle diagnostics (-vv also passes agent stderr through).
        #[arg(long, short = 'v', action = clap::ArgAction::Count)]
        verbose: u8,
        /// Disable ANSI colors; keep text prefixes.
        #[arg(long)]
        no_color: bool,
        /// Ask provider for ptah.ask: `stdin` or `none`. Overrides
        /// PTAH_ASK, the registry [ask] section, and auto-detection.
        #[arg(long, value_name = "PROVIDER", value_parser = parse_ask_provider)]
        ask: Option<AskProviderKind>,
    },

    /// Verify a script without executing it.
    Check {
        /// Path to the entry Luau script.
        script: PathBuf,
        /// Disable ANSI coloring of findings.
        #[arg(long)]
        no_color: bool,
        /// Ask provider the interaction lint resolves against:
        /// `stdin` or `none`. Same precedence as `run --ask`.
        #[arg(long, value_name = "PROVIDER", value_parser = parse_ask_provider)]
        ask: Option<AskProviderKind>,
    },

    /// Print the Luau type definitions for the ptah script API.
    Types,

    /// Print a shell completion script for the ptah CLI.
    Completions {
        /// Shell to generate a completion script for.
        shell: Shell,
    },

    /// Scaffold ./.ptah/ (type definitions + agent registry skeleton).
    Init,

    /// Hidden: MCP bridge server for typed results. Spawned per result
    /// session by the agent, as suggested in `session/new { mcpServers }`;
    /// not part of the user-facing surface.
    #[command(name = "__bridge", hide = true)]
    Bridge,
}

/// What `Cli::try_parse_from` produced: dispatch early on subcommands that
/// need no runtime setup.
#[derive(Debug)]
enum Parsed {
    /// `ptah run` — full orchestration run.
    Run {
        script: PathBuf,
        render: RenderOptions,
        verbose: u8,
        ask: Option<AskProviderKind>,
    },
    /// `ptah types` — print definitions, exit 0, touch nothing else.
    Types,
    /// `ptah completions <shell>` — print a completion script generated
    /// from the live command tree, exit 0, touch nothing else.
    Completions(Shell),
    /// `ptah init` — scaffold ./.ptah/ (definitions + registry
    /// skeleton), exit 0 unless writing fails.
    Init,
    /// `ptah check` — verify a script without executing it.
    Check {
        script: PathBuf,
        no_color: bool,
        ask: Option<AskProviderKind>,
    },
    /// `ptah __bridge` — typed-results MCP server over stdio.
    Bridge,
}

/// Parse CLI arguments (unit-testable).
fn parse(args: &[String]) -> Result<Parsed, clap::Error> {
    let cli = Cli::try_parse_from(args)?;
    Ok(match cli.command {
        Command::Run {
            script,
            quiet,
            verbose,
            no_color,
            ask,
        } => Parsed::Run {
            script,
            render: RenderOptions {
                quiet,
                no_color,
                verbose: verbose >= 1,
                agent_stderr: verbose >= 2,
            },
            verbose,
            ask,
        },
        Command::Types => Parsed::Types,
        Command::Completions { shell } => Parsed::Completions(shell),
        Command::Init => Parsed::Init,
        Command::Check {
            script,
            no_color,
            ask,
        } => Parsed::Check {
            script,
            no_color,
            ask,
        },
        Command::Bridge => Parsed::Bridge,
    })
}

/// `ptah types`: print the definitions with a version header. Everything
/// after line 1 is byte-identical to `.ptah/ptah.d.luau`. Requires no
/// script, registry, or agent configuration.
fn print_types() -> ExitCode {
    print!("{}", definitions_bytes());
    ExitCode::SUCCESS
}

/// The definitions exactly as `ptah types` prints them and exactly as
/// `ptah init` writes `.ptah/ptah.d.luau`: one version-header line plus
/// the embedded file (which ends with a trailing newline), so the
/// command's stdout and the scaffolded file are byte-identical by
/// construction.
fn definitions_bytes() -> String {
    format!(
        "-- ptah {} type definitions\n{}",
        crate::VERSION,
        crate::check::defs::TYPE_DEFINITIONS
    )
}

/// `ptah completions <shell>`: print the completion script for the named
/// shell and nothing else. Generated from this binary's own command tree
/// so the emitted script always matches the installed surface. Requires no
/// script, registry, or agent configuration and never touches the
/// filesystem.
fn print_completions(shell: Shell) -> ExitCode {
    let mut cmd = completion_command();
    clap_complete::generate(shell, &mut cmd, "ptah", &mut std::io::stdout());
    ExitCode::SUCCESS
}

/// The completion source tree: the live `Cli` command with hidden
/// subcommands stripped. clap_complete does not omit `hide`-flagged
/// subcommands (verified against 4.6), so the internal `__bridge`
/// command would otherwise leak into every emitted script. Everything
/// else — args, help text, new visible subcommands — still derives from
/// the live struct, so emitted scripts cannot drift from the binary.
fn completion_command() -> clap::Command {
    let mut full = Cli::command();
    // `Cli` pins `name = "ptah"` and derives `version` from the
    // `&'static` `VERSION` const; clap's `Str` accepts only `'static`
    // strings, so the rebuild re-states both rather than borrowing off
    // `full`.
    let mut visible = clap::Command::new("ptah")
        .version(crate::VERSION)
        // Mirrors what clap_derive generates for the required subcommand
        // field of `Cli`.
        .subcommand_required(true)
        .arg_required_else_help(true);
    if let Some(about) = full.get_about().cloned() {
        visible = visible.about(about);
    }
    for sub in full.get_subcommands_mut() {
        if !sub.is_hide_set() {
            visible = visible.subcommand(sub.clone());
        }
    }
    visible
}

/// `.ptah/config.toml` scaffold written by `ptah init`: a fully
/// commented registry skeleton. Parses as a valid empty registry
/// exactly as written (pinned by test), so a fresh scaffold is a
/// working (agentless) registry from the first run.
const CONFIG_SKELETON: &str = r#"# ptah agent registry (project layer).
#
# Discovery: ptah looks for .ptah/config.toml in the directory it runs
# in and every parent directory, and also reads the user layer
# ($XDG_CONFIG_HOME/ptah/config.toml or ~/.config/ptah/config.toml).
# When both layers define the same agent name the project entry wins,
# wholesale; agents defined in only one layer pass through.
#
# Fields per agent:
#   command  required — the executable to spawn
#   args     optional — argv after the command (default: none)
#   env      optional — extra environment for the child; `${VAR}`
#            interpolates from ptah's environment at resolve time
#            (unset becomes empty) and values merge over the inherited
#            environment.
#
# Example — uncomment and edit:
#
# [agents.claude]
# command = "npx"
# args = ["-y", "@agentclientprotocol/claude-agent-acp@latest"]
#
# [agents.claude.env]
# ANTHROPIC_API_KEY = "${ANTHROPIC_API_KEY}"
#
# ------------------------------------------------------------------
# Interaction ([ask] section, optional, first global section): the
# provider ptah.ask pauses the run to ask a human. `stdin` renders
# prompts on stdout and reads answers from stdin; `none` prohibits
# asking outright. Selection precedence: --ask flag > PTAH_ASK env >
# project [ask] > user [ask] > auto-detect (stdin only when both
# stdin and stdout are terminals). With both layers defining [ask],
# the project section wins wholesale.
#
# [ask]
# provider = "stdin"
"#;

/// Next-step hints printed by `ptah init` on every run — including
/// runs where nothing changed.
const INIT_HINTS: &str = r#"Next steps:

  1. Edit .ptah/config.toml — add an agent under [agents.<name>]; the
     comments there document every field and the two-layer discovery.
  2. Point your editor's luau-lsp at .ptah/ptah.d.luau (platform
     "standard") for script completion and type checking — see the
     README "Editor setup" section. After upgrading ptah, re-run
     ptah init to refresh the definitions; the alternative (scripting,
     or refreshing without touching config) is:
     ptah types > .ptah/ptah.d.luau
  3. Install CLI completions for your shell — ptah completions <shell>;
     per-shell install lines are in the README "Shell completions"
     section.
  4. Scripting ptah from a coding agent? The ptah skill documents the
     whole API: skills/ptah/SKILL.md in the ptah repo.
"#;

/// Parse a ptah definitions version header — exactly the shape
/// `definitions_bytes()` emits: `-- ptah <version> type definitions`
/// with a non-empty version (design D3). Strict by construction: an
/// exact prefix/suffix match with a non-empty middle; near-miss text,
/// leading/trailing junk, and an empty version all read as
/// unparseable. A malformed header never blocks an overwrite — it
/// only costs the `updated:` line its version suffix.
fn parse_defs_header(line: &str) -> Option<String> {
    let version = line
        .strip_prefix("-- ptah ")?
        .strip_suffix(" type definitions")?;
    if version.is_empty() {
        return None;
    }
    Some(version.to_string())
}

/// Sync the project definitions (the `.ptah/ptah.d.luau` half of
/// `ptah init`): a derived artifact of the installed binary, not
/// user config. Created when absent; overwritten whenever its bytes
/// differ from the current emit; left untouched only when it already
/// matches one of the two accepted forms byte-for-byte (design D1) —
/// the emitted output itself, or the emitted output minus its header
/// line (design D2, so this repository's own headerless source
/// definitions report current instead of being rewritten with a
/// prepended header that would double at the next build). Overwrite
/// is otherwise unconditional: provenance is not checked, only
/// bytes. The write is a plain `fs::write` (design D5), matching the
/// created path. Returns the one message line for the file; I/O
/// failures propagate to the caller's exit-1 path.
fn sync_definitions(path: &str) -> std::io::Result<String> {
    let emitted = definitions_bytes();
    let existing = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(path, &emitted)?;
            return Ok(format!("created: {path}"));
        }
        Err(e) => return Err(e),
    };
    if existing == emitted.as_bytes() || existing == crate::check::defs::TYPE_DEFINITIONS.as_bytes()
    {
        return Ok(format!("up to date: {path}"));
    }
    // Design D3: the version suffix comes from the overwritten file's
    // first line when it parses as a ptah header — a stale emit gets
    // `updated: (old -> new)`, a hand-edited or foreign file a bare
    // `updated:`. An unparseable first line (or invalid UTF-8) never
    // blocks the overwrite, only the suffix.
    let old = existing
        .split(|&b| b == b'\n')
        .next()
        .and_then(|line| std::str::from_utf8(line).ok())
        .and_then(parse_defs_header);
    let current = crate::VERSION;
    std::fs::write(path, &emitted)?;
    Ok(match old {
        Some(old) => format!("updated: {path} ({old} -> {current})"),
        None => format!("updated: {path}"),
    })
}

/// `ptah init`: scaffold `./.ptah/` in the current working directory
/// with exactly two files — `ptah.d.luau` (byte-identical to
/// `ptah types` stdout) and `config.toml` (commented registry
/// skeleton). The two have different ownership: the config is
/// user-authored — created when absent, skipped with a message
/// otherwise, never modified — while the definitions are a derived
/// artifact this binary syncs via `sync_definitions` (created /
/// updated / confirmed current), so re-running after an upgrade is
/// the refresh path and a partial scaffold still completes. Hints
/// print on every run. Requires no script, registry, or agent
/// configuration; a failure to create the directory or write a file
/// reports on stderr and exits 1.
fn run_init() -> ExitCode {
    if let Err(e) = std::fs::create_dir_all("./.ptah") {
        eprintln!("error: cannot create ./.ptah: {e}");
        return ExitCode::from(1);
    }
    // The config keeps its create-or-skip semantics, unchanged.
    if std::path::Path::new(".ptah/config.toml").exists() {
        println!("skipped (exists): .ptah/config.toml");
    } else if let Err(e) = std::fs::write(".ptah/config.toml", CONFIG_SKELETON) {
        eprintln!("error: cannot write .ptah/config.toml: {e}");
        return ExitCode::from(1);
    } else {
        println!("created: .ptah/config.toml");
    }
    match sync_definitions(".ptah/ptah.d.luau") {
        Ok(line) => println!("{line}"),
        Err(e) => {
            eprintln!("error: cannot write .ptah/ptah.d.luau: {e}");
            return ExitCode::from(1);
        }
    }
    print!("{INIT_HINTS}");
    ExitCode::SUCCESS
}

/// Resolve the run's interaction posture exactly once, under the full
/// selection chain: `--ask` flag > `PTAH_ASK` environment > merged
/// registry `[ask]` section > auto-detection (stdin provider only when
/// both stdin and stdout are terminals). `Err` carries the invalid
/// `PTAH_ASK` value message — a configuration failure the callers
/// report with exit 2, like registry discovery failures. Unit-testable
/// with injected layers; the provider is built here (the one place
/// that touches the world).
fn resolve_interaction(
    flag: Option<AskProviderKind>,
    env: Option<&str>,
    section: Option<AskProviderKind>,
    interactive: bool,
) -> Result<InteractionMode, String> {
    let from_env = match env {
        None => None,
        Some("") => None,
        Some(raw) => Some(
            AskProviderKind::from_str(raw).map_err(|e| format!("invalid PTAH_ASK value: {e}"))?,
        ),
    };
    let kind = flag.or(from_env).or(section).or_else(|| {
        // Auto-detection: stdin only on a fully interactive terminal.
        interactive.then_some(AskProviderKind::Stdin)
    });
    Ok(match kind {
        Some(AskProviderKind::Stdin) => {
            InteractionMode::Provider(Arc::new(StdinAskProvider::new()))
        }
        Some(AskProviderKind::None) => InteractionMode::Prohibited,
        None => InteractionMode::Unresolved,
    })
}

/// `ptah check`: compile + static lints in-process, then the luau-lsp
/// typecheck pass with the embedded definitions. Exit `0` clean, `1`
/// findings, `2` the check could not run (missing script, registry
/// discovery failure, invalid `PTAH_ASK`, luau-lsp missing).
fn run_check(script: PathBuf, no_color: bool, ask: Option<AskProviderKind>) -> ExitCode {
    if !script.is_file() {
        eprintln!("error: script not found: {}", script.display());
        return ExitCode::from(2);
    }
    let invocation_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let registry = match crate::config_fs::FsConfigSource.discover(&invocation_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let interaction = match resolve_interaction(
        ask,
        std::env::var("PTAH_ASK").ok().as_deref(),
        registry.ask().map(|a| a.provider),
        stdin_is_tty() && stdout_is_tty(),
    ) {
        Ok(mode) => mode,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let code = crate::check::check(&crate::check::CheckConfig {
        script_path: script,
        registry,
        interaction,
        color: !no_color,
    });
    ExitCode::from(code)
}

/// Entry point: returns the process exit code.
pub fn main() -> ExitCode {
    let mut args: Vec<String> = vec!["ptah".to_string()];
    args.extend(std::env::args().skip(1));
    let (script, render_opts, verbose, ask) = match parse(&args) {
        Ok(Parsed::Run {
            script,
            render,
            verbose,
            ask,
        }) => (script, render, verbose, ask),
        Ok(Parsed::Types) => return print_types(),
        Ok(Parsed::Completions(shell)) => return print_completions(shell),
        Ok(Parsed::Init) => return run_init(),
        Ok(Parsed::Check {
            script,
            no_color,
            ask,
        }) => return run_check(script, no_color, ask),
        Ok(Parsed::Bridge) => return crate::bridge::run(),
        Err(e) => {
            // --help / --version are "errors" that carry their own output
            // and exit code.
            e.print().ok();
            let kind = e.kind();
            if matches!(
                kind,
                clap::error::ErrorKind::DisplayVersion | clap::error::ErrorKind::DisplayHelp
            ) {
                return ExitCode::SUCCESS;
            }
            return ExitCode::from(2);
        }
    };

    if !script.is_file() {
        eprintln!("error: script not found: {}", script.display());
        return ExitCode::from(2);
    }

    // Tracing to stderr for library internals; the renderer owns stdout.
    let level = match verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
        )
        .with_ansi(!render_opts.no_color)
        .with_writer(std::io::stderr)
        .try_init();

    let invocation_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let registry = match crate::config_fs::FsConfigSource.discover(&invocation_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    // Interaction posture: resolved once, before pre-flight and agents
    // (an invalid PTAH_ASK is a configuration failure — exit 2, the
    // discovery-failure class). The resolved mode feeds both consumers:
    // the pre-flight interaction lint and the run binding.
    let interaction = match resolve_interaction(
        ask,
        std::env::var("PTAH_ASK").ok().as_deref(),
        registry.ask().map(|a| a.provider),
        stdin_is_tty() && stdout_is_tty(),
    ) {
        Ok(mode) => mode,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    // Pre-flight: fail certainly-broken scripts (uncompilable entry or
    // reachable module, unresolvable literal require, unknown literal
    // agent name, unusable ptah.ask posture) before anything spawns.
    // Computed forms are not linted; strictness is not enforced here
    // (`ptah check` does that).
    let preflight = crate::check::preflight(&script, &registry, &interaction);
    if !preflight.is_empty() {
        for finding in &preflight {
            eprintln!("{}", finding.render(!render_opts.no_color));
        }
        eprintln!("{}", crate::check::summary_line(&preflight));
        return ExitCode::from(1);
    }

    let renderer = std::sync::Arc::new(Renderer::new(render_opts));
    // The composition line change ② moved out of the script crate: the
    // ACP stdio adapter is chosen here, at the composition root, and
    // injected through the `AgentTransport` port. The process runner is
    // the same decision one port over: ptah always injects it (there is
    // no gating flag — running a ptah script already implies arbitrary
    // shell through the headless allow-all agent posture).
    // Outer cancellation channel: SIGINT/SIGTERM forward into it below
    // (first signal — teardown rides inside `run`, exit code 130/143;
    // second signal — kill every registered child group, then exit at
    // once). Created before the runtime so the config is complete
    // here; the monitor task is spawned inside it.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(0);
    // The run's child-group registry: agent sessions (via the
    // transport) and exec children (via the process runner) register
    // their group-leader pids here, and only the second signal ever
    // reads it — teardown keeps its own kill paths. One instance, one
    // run: this is the map the force escape sweeps.
    let groups = std::sync::Arc::new(ptah_core::groups::ProcessGroups::new());
    let config = RunConfig {
        script_path: script,
        invocation_dir,
        registry,
        transport: std::sync::Arc::new(ptah_acp::Transport::with_registry(groups.clone())),
        process_runner: Some(std::sync::Arc::new(
            crate::exec::TokioProcessRunner::with_registry(groups.clone()),
        )),
        interaction,
        shutdown: Some(shutdown_rx),
        renderer,
        // The env snapshot behind `os.getenv`: captured once here via
        // `vars_os` + UTF-8 filter — `env::vars()` panics on non-UTF-8
        // entries, while those entries merely read as unset (design D5).
        env: std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .collect(),
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    // Outer cancellation: SIGINT/SIGTERM race the script inside `run`.
    // The first signal rides the run's teardown path (kills in-flight
    // exec process groups, closes agent sessions) and the run reports
    // the shell-conventional 128+signal exit code; a second signal
    // during teardown exits at once. Signal disposition is a
    // world-touching composition decision, so it is installed here —
    // the run loop itself only sees the channel.
    let outcome = rt.block_on(async {
        let signal_monitor = install_signal_monitor(shutdown_tx, groups);
        let outcome = tokio::task::LocalSet::new()
            .run_until(script::run(config))
            .await;
        if let Some(monitor) = signal_monitor {
            monitor.abort();
        }
        outcome
    });

    if let Some(error) = &outcome.error {
        eprintln!("error: {error}");
    }
    for err in &outcome.undelivered_errors {
        eprintln!("error: unobserved task error: {err}");
    }

    // End the runtime without Runtime::drop's wait for blocking tasks:
    // tokio::io::Stdin routes reads through the blocking pool, and a
    // cancelled ask drops only the read *future* — the pool thread sits
    // in read(2) until the stdin writer closes. Waiting for it (drop's
    // documented behavior) would hold a SIGINT-during-ask exit hostage
    // on an interactive stdin that simply never closed. Everything
    // else is already torn down inside `run` (teardown joins sessions
    // and kills exec groups; the signal monitor is aborted above).
    rt.shutdown_background();
    ExitCode::from(u8::try_from(outcome.code).unwrap_or(1))
}

/// Install the SIGINT/SIGTERM monitor forwarding into the run's
/// shutdown watch. The first signal sends the shell-conventional code
/// (130/143) — the run then tears itself down — and a second signal
/// during that teardown kills every process group still in `groups`
/// (agent and exec children alike) before the immediate exit, with
/// the exit code matching *that* signal. Returns the monitor task,
/// aborted when the run ends on its own so a late signal cannot kill
/// a finishing process. Non-unix platforms have no such signals: no
/// monitor, and a dropped sender the run reads as "nobody will ever
/// cancel".
fn install_signal_monitor(
    shutdown_tx: tokio::sync::watch::Sender<i32>,
    groups: std::sync::Arc<ptah_core::groups::ProcessGroups>,
) -> Option<tokio::task::JoinHandle<()>> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let Ok(mut int) = signal(SignalKind::interrupt()) else {
            return None;
        };
        let Ok(mut term) = signal(SignalKind::terminate()) else {
            return None;
        };
        Some(tokio::spawn(async move {
            let code = tokio::select! {
                _ = int.recv() => 130,
                _ = term.recv() => 143,
            };
            let _ = shutdown_tx.send(code);
            // A second signal means the user wants out *now*: teardown
            // is still draining and no destructor will run on the hard
            // exit — kill every registered child group first, then exit
            // with the code of the signal that fired (a second SIGTERM
            // reports 143, not the old hardcoded 130).
            let second = tokio::select! {
                _ = int.recv() => 130,
                _ = term.recv() => 143,
            };
            kill_registered_groups(&groups);
            std::process::exit(second);
        }))
    }
    #[cfg(not(unix))]
    {
        drop(shutdown_tx);
        drop(groups);
        None
    }
}

/// SIGKILL every process group in the registry's snapshot: the second
/// signal's escape hatch. Raw and idempotent — an entry whose group
/// is already dead yields `ESRCH`, ignored — and deliberately
/// synchronous: this must run exactly when the async runtime is too
/// wedged to drain. No reap: the killed groups are re-parented to
/// init, which reaps them; nothing observable leaks.
fn kill_registered_groups(groups: &ptah_core::groups::ProcessGroups) {
    for pid in groups.snapshot() {
        #[cfg(unix)]
        // SAFETY: raw kill(2) on pids this run spawned as group
        // leaders; a stale entry only yields ESRCH (no-op).
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("ptah".to_string())
            .chain(list.iter().map(std::string::ToString::to_string))
            .collect()
    }

    #[test]
    fn types_subcommand_parses_without_run_arguments() {
        match parse(&args(&["types"])).unwrap() {
            Parsed::Types => {}
            Parsed::Run { .. }
            | Parsed::Check { .. }
            | Parsed::Completions(_)
            | Parsed::Init
            | Parsed::Bridge => panic!("expected Types"),
        }
    }

    #[test]
    fn completions_subcommand_parses_every_shell() {
        for (name, shell) in [
            ("bash", Shell::Bash),
            ("zsh", Shell::Zsh),
            ("fish", Shell::Fish),
            ("elvish", Shell::Elvish),
            ("powershell", Shell::PowerShell),
        ] {
            match parse(&args(&["completions", name])).unwrap() {
                Parsed::Completions(s) => assert_eq!(s, shell, "mismatch for {name}"),
                _ => panic!("expected Completions for {name}"),
            }
        }
    }

    #[test]
    fn completions_unknown_shell_is_a_usage_error() {
        let err = parse(&args(&["completions", "tcsh"])).unwrap_err();
        assert!(!matches!(
            err.kind(),
            clap::error::ErrorKind::DisplayVersion | clap::error::ErrorKind::DisplayHelp
        ));
    }

    #[test]
    fn completions_missing_shell_is_a_usage_error() {
        let err = parse(&args(&["completions"])).unwrap_err();
        assert!(!matches!(
            err.kind(),
            clap::error::ErrorKind::DisplayVersion | clap::error::ErrorKind::DisplayHelp
        ));
    }

    #[test]
    fn completion_tree_carries_visible_subcommands_and_hides_bridge() {
        let cmd = completion_command();
        let names: Vec<String> = cmd
            .get_subcommands()
            .map(|c| c.get_name().to_string())
            .collect();
        for visible in ["run", "check", "types", "completions", "init"] {
            assert!(names.iter().any(|n| n == visible), "{visible} missing");
        }
        assert!(!names.iter().any(|n| n == "__bridge"), "{names:?}");
    }

    #[test]
    fn init_subcommand_parses() {
        match parse(&args(&["init"])).unwrap() {
            Parsed::Init => {}
            _ => panic!("expected Init"),
        }
    }

    #[test]
    fn init_skeleton_parses_as_an_empty_registry() {
        let registry = crate::config_fs::from_parts(None, Some(CONFIG_SKELETON)).unwrap();
        assert!(
            registry.agent_names().is_empty(),
            "skeleton must parse with no agents: {:?}",
            registry.agent_names()
        );
    }

    #[test]
    fn definitions_bytes_are_header_plus_embedded_file() {
        let bytes = definitions_bytes();
        let (header, body) = bytes
            .split_once('\n')
            .unwrap_or_else(|| panic!("no header line: {bytes:?}"));
        assert_eq!(
            header,
            format!("-- ptah {} type definitions", crate::VERSION)
        );
        assert_eq!(
            body,
            crate::check::defs::TYPE_DEFINITIONS,
            "definitions body must be the embedded file byte-for-byte"
        );
    }

    #[test]
    fn parse_defs_header_accepts_the_emitted_shape() {
        assert_eq!(
            parse_defs_header("-- ptah 0.0.1 type definitions"),
            Some("0.0.1".to_string())
        );
        assert_eq!(
            parse_defs_header(&format!("-- ptah {} type definitions", crate::VERSION)),
            Some(crate::VERSION.to_string())
        );
    }

    #[test]
    fn parse_defs_header_rejects_empty_version_and_near_misses() {
        // Empty version: the double space collapses both delimiters.
        assert_eq!(parse_defs_header("-- ptah  type definitions"), None);
        // Near-miss text: none of these is the exact emitted shape.
        for line in [
            "-- ptah 0.1.0 type definition",    // missing plural
            "-- ptah 0.1.0 definitions",        // missing middle words
            "-- ptahx 0.1.0 type definitions",  // prefix not exact
            "--  ptah 0.1.0 type definitions",  // double space after --
            "  -- ptah 0.1.0 type definitions", // leading whitespace
            "-- ptah 0.1.0 type definitions ",  // trailing whitespace
            "--ptah 0.1.0 type definitions",    // missing space
            "",                                 // nothing at all
        ] {
            assert_eq!(parse_defs_header(line), None, "must reject {line:?}");
        }
    }

    #[test]
    fn sync_definitions_creates_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ptah.d.luau");
        let line = sync_definitions(path.to_str().unwrap()).unwrap();
        assert_eq!(line, format!("created: {}", path.display()));
        assert_eq!(
            std::fs::read(&path).unwrap(),
            definitions_bytes().as_bytes(),
            "created file must be the current emit byte-for-byte"
        );
    }

    #[test]
    fn sync_definitions_reports_up_to_date_for_both_accepted_forms() {
        // Design D1/D2: both the emitted form and the headerless body
        // verbatim (this repo's source layout) are current — no write.
        for existing in [
            definitions_bytes(),
            crate::check::defs::TYPE_DEFINITIONS.to_string(),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("ptah.d.luau");
            std::fs::write(&path, &existing).unwrap();
            let line = sync_definitions(path.to_str().unwrap()).unwrap();
            assert_eq!(
                line,
                format!("up to date: {}", path.display()),
                "for existing {existing:?}"
            );
            assert_eq!(
                std::fs::read(&path).unwrap(),
                existing.as_bytes(),
                "no-write outcome must leave the file untouched"
            );
        }
    }

    #[test]
    fn sync_definitions_updates_stale_file_with_version_arrow() {
        // A fabricated older emit: ptah header naming an old version
        // plus a body that differs from the current one.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ptah.d.luau");
        std::fs::write(&path, "-- ptah 0.0.1 type definitions\n-- stale body\n").unwrap();
        let line = sync_definitions(path.to_str().unwrap()).unwrap();
        assert_eq!(
            line,
            format!("updated: {} (0.0.1 -> {})", path.display(), crate::VERSION)
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            definitions_bytes().as_bytes(),
            "stale file must be overwritten with the current emit"
        );
    }

    #[test]
    fn sync_definitions_overwrites_headerless_difference_without_suffix() {
        // Hand-edited or foreign content: differs, first line is not a
        // ptah header — overwritten all the same, updated: line bare.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ptah.d.luau");
        std::fs::write(&path, "--!strict\n-- hand-edited or foreign content\n").unwrap();
        let line = sync_definitions(path.to_str().unwrap()).unwrap();
        assert_eq!(line, format!("updated: {}", path.display()));
        assert_eq!(
            std::fs::read(&path).unwrap(),
            definitions_bytes().as_bytes(),
            "differing file must be overwritten with the current emit"
        );
    }

    #[test]
    fn missing_script_argument_is_a_usage_error() {
        let err = parse(&args(&["run"])).unwrap_err();
        assert!(!matches!(
            err.kind(),
            clap::error::ErrorKind::DisplayVersion | clap::error::ErrorKind::DisplayHelp
        ));
    }

    #[test]
    fn check_subcommand_parses() {
        match parse(&args(&["check", "s.luau"])).unwrap() {
            Parsed::Check {
                script, no_color, ..
            } => {
                assert_eq!(script, PathBuf::from("s.luau"));
                assert!(!no_color);
            }
            _ => panic!("expected Check"),
        }
        match parse(&args(&["check", "s.luau", "--no-color"])).unwrap() {
            Parsed::Check { no_color: true, .. } => {}
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn ask_flag_parses_on_run_and_check() {
        // Both spellings, both subcommands, both v1 providers.
        for form in [["--ask=stdin"].as_slice(), ["--ask", "stdin"].as_slice()] {
            let argv = ["run", "s.luau"]
                .iter()
                .chain(form.iter())
                .copied()
                .collect::<Vec<_>>();
            match parse(&args(&argv)).unwrap() {
                Parsed::Run {
                    ask: Some(kind), ..
                } => assert_eq!(kind, AskProviderKind::Stdin),
                _ => panic!("expected Run with ask"),
            }
        }
        match parse(&args(&["check", "s.luau", "--ask=none"])).unwrap() {
            Parsed::Check {
                ask: Some(kind), ..
            } => assert_eq!(kind, AskProviderKind::None),
            _ => panic!("expected Check with ask"),
        }
        // Absent flag stays None (selection falls through the chain).
        match parse(&args(&["run", "s.luau"])).unwrap() {
            Parsed::Run { ask: None, .. } => {}
            _ => panic!("expected Run without ask"),
        }
    }

    #[test]
    fn ask_flag_rejects_unknown_providers_with_a_usage_error() {
        let err = parse(&args(&["run", "--ask=webhook", "s.luau"])).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
        let msg = err.to_string();
        assert!(msg.contains("webhook"), "names the value: {msg}");
        assert!(
            msg.contains("stdin") && msg.contains("none"),
            "names accepted values: {msg}"
        );
        let err = parse(&args(&["check", "--ask=webhook", "s.luau"])).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
    }

    #[test]
    fn check_without_script_is_a_usage_error() {
        let err = parse(&args(&["check"])).unwrap_err();
        assert!(!matches!(
            err.kind(),
            clap::error::ErrorKind::DisplayVersion | clap::error::ErrorKind::DisplayHelp
        ));
    }

    #[test]
    fn missing_subcommand_is_a_usage_error() {
        let err = parse(&args(&[])).unwrap_err();
        assert!(!matches!(
            err.kind(),
            clap::error::ErrorKind::DisplayVersion | clap::error::ErrorKind::DisplayHelp
        ));
    }

    #[test]
    fn flags_parse() {
        let Parsed::Run {
            script,
            render: opts,
            verbose: v,
            ..
        } = parse(&args(&["run", "s.luau"])).unwrap()
        else {
            panic!("expected Run")
        };
        assert_eq!(script, PathBuf::from("s.luau"));
        assert!(!opts.quiet && !opts.no_color && !opts.verbose && !opts.agent_stderr);
        assert_eq!(v, 0);

        let Parsed::Run {
            render: opts,
            verbose: v,
            ..
        } = parse(&args(&["run", "s.luau", "--quiet", "--no-color"])).unwrap()
        else {
            panic!("expected Run")
        };
        assert!(opts.quiet && opts.no_color);
        assert_eq!(v, 0);

        let Parsed::Run {
            render: opts,
            verbose: v,
            ..
        } = parse(&args(&["run", "s.luau", "-vv"])).unwrap()
        else {
            panic!("expected Run")
        };
        assert!(opts.verbose && opts.agent_stderr);
        assert_eq!(v, 2);
    }

    #[test]
    fn version_flag_reports_version() {
        // clap handles --version at parse time; a successful parse of
        // --version is a DisplayVersion "error".
        let parsed = Cli::try_parse_from(args(&["--version"]));
        assert!(parsed.is_err()); // clap exits with the version print
        let err = parsed.unwrap_err().to_string();
        assert!(
            err.contains(crate::VERSION) || err.contains("version"),
            "{err}"
        );
    }

    /// The second-signal sweep, pinned on a real child: spawn `sleep`
    /// as its own process-group leader (exactly how both the ACP
    /// transport and the exec runner spawn children), register it,
    /// sweep, assert the kill landed. No async anywhere — the sweep
    /// must work precisely when the runtime is too wedged to drain.
    #[cfg(unix)]
    #[test]
    fn sweep_kills_registered_process_groups() {
        use std::os::unix::process::CommandExt;

        let groups = ptah_core::groups::ProcessGroups::new();
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .expect("spawn sleep");
        groups.register(child.id());

        kill_registered_groups(&groups);

        let status = child.wait().expect("wait sleep");
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            status.signal(),
            Some(libc::SIGKILL),
            "sweep must SIGKILL the registered group: {status:?}"
        );
        // A swept (dead) entry is harmless bookkeeping, not an error.
        assert_eq!(groups.snapshot(), vec![child.id()]);
        kill_registered_groups(&groups); // idempotent: ESRCH ignored
        groups.deregister(child.id());
        assert!(groups.snapshot().is_empty());
    }

    // ------------------------------------------------------------------
    // Interaction-mode resolution (ask capability: precedence chain)
    // ------------------------------------------------------------------

    #[test]
    fn interaction_resolution_follows_the_precedence_chain() {
        use ptah_core::ports::InteractionMode::*;

        let stdin = Some(AskProviderKind::Stdin);
        let none = Some(AskProviderKind::None);

        // No overrides anywhere: auto-detect decides.
        assert!(matches!(
            resolve_interaction(None, None, None, true).unwrap(),
            Provider(_)
        ));
        assert!(matches!(
            resolve_interaction(None, None, None, false).unwrap(),
            Unresolved
        ));

        // Flag beats environment (even when they disagree).
        assert!(matches!(
            resolve_interaction(stdin, Some("none"), none, false).unwrap(),
            Provider(_)
        ));
        assert!(matches!(
            resolve_interaction(none, Some("stdin"), stdin, false).unwrap(),
            Prohibited
        ));

        // Environment beats the registry section.
        assert!(matches!(
            resolve_interaction(None, Some("none"), stdin, false).unwrap(),
            Prohibited
        ));
        assert!(matches!(
            resolve_interaction(None, Some("stdin"), none, false).unwrap(),
            Provider(_)
        ));

        // Section beats auto-detect; section alone applies.
        assert!(matches!(
            resolve_interaction(None, None, none, true).unwrap(),
            Prohibited
        ));
        assert!(matches!(
            resolve_interaction(None, None, stdin, false).unwrap(),
            Provider(_)
        ));
    }

    #[test]
    fn interaction_resolution_rejects_invalid_env_value() {
        let err = resolve_interaction(None, Some("webhook"), None, true).unwrap_err();
        assert!(err.contains("PTAH_ASK"), "names the variable: {err}");
        assert!(
            err.contains("stdin") && err.contains("none"),
            "names accepted values: {err}"
        );
        // Valid values and an unset/empty variable are fine.
        assert!(resolve_interaction(None, Some("stdin"), None, false).is_ok());
        assert!(resolve_interaction(None, Some(""), None, false).is_ok());
    }
}
