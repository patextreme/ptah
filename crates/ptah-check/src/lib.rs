//! `ptah check`: zero-execution script verification.
//!
//! Three passes over a script and its literal require graph — an
//! in-process compile pass (mlua, the same compiler `run` uses), static
//! lints over the full-moon AST (agent names, require targets,
//! `--!strict` directives), and a typecheck pass that shells out to
//! `luau-lsp analyze` with the binary's embedded definitions. Nothing in
//! this module executes script code: the entry chunk is compiled with
//! [`mlua::Chunk::into_function`] and never called, no required module
//! loads, no agent subprocess launches.
//!
//! The same lint machinery backs the `run` pre-flight
//! ([`preflight`]), narrowed to compile + require + agent checks.

pub mod defs;
pub(crate) mod lint;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ptah_core::config::Registry;
use ptah_core::ports::InteractionMode;

use crate::lint::ParsedFile;

/// One in-process finding: a real path, a 1-based position, a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: PathBuf,
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl Finding {
    /// Render as `path:line:col: message`, with an ANSI-colored prefix
    /// unless `color` is false.
    pub fn render(&self, color: bool) -> String {
        let prefix = format!("{}:{}:{}:", self.path.display(), self.line, self.column);
        if color {
            format!("\x1b[1;31m{prefix}\x1b[0m {}", self.message)
        } else {
            format!("{prefix} {}", self.message)
        }
    }
}

/// Summary line: the number of findings and files affected.
pub fn summary_line(findings: &[Finding]) -> String {
    let files = findings
        .iter()
        .map(|f| &f.path)
        .collect::<HashSet<_>>()
        .len();
    format!(
        "found {} finding{} in {} file{}",
        findings.len(),
        if findings.len() == 1 { "" } else { "s" },
        files,
        if files == 1 { "" } else { "s" }
    )
}

/// Configuration for one `ptah check`.
pub struct CheckConfig {
    pub script_path: PathBuf,
    pub registry: Registry,
    /// The resolved interaction posture: with ask call sites present,
    /// `Prohibited` and `Unresolved` are findings; with none, the
    /// posture produces nothing. Resolved by the CLI (which owns
    /// terminal detection); the check itself stays free of process
    /// context.
    pub interaction: InteractionMode,
    /// ANSI-color the in-process findings.
    pub color: bool,
}

/// Run the full check pipeline (all passes, findings collected, never
/// fail-fast) and report on stderr. Returns the exit code: `0` clean,
/// `1` findings, `2` the check could not run.
pub fn check(cfg: &CheckConfig) -> u8 {
    let entry = std::fs::canonicalize(&cfg.script_path).unwrap_or_else(|_| cfg.script_path.clone());
    let source = match std::fs::read_to_string(&entry) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read script {}: {e}", entry.display());
            return 2;
        }
    };

    let mut findings = compile_findings(&[(entry.clone(), source)]);

    // Static lints over the literal require graph.
    let walked = lint::walk(&entry);
    let entry_compiled = findings.is_empty();
    // The entry's syntax errors are already reported by the compile pass
    // (mlua is the compiler `run` uses); drop a duplicate full-moon
    // parse finding for the entry when both fire.
    findings.extend(
        walked
            .failures
            .into_iter()
            .filter(|f| entry_compiled || f.path != entry),
    );
    findings.extend(walked.broken);

    // Strict-directive lint: the entry and every reachable *authored*
    // file. Generated package artifacts under a `luau_packages`
    // directory (pesde's linker modules and container contents) are
    // machine-written without directives and are exempt — installed
    // packages must check clean through their aliases.
    for file in &walked.parsed {
        if !file.strict && !under_luau_packages(&file.path) {
            findings.push(Finding {
                path: file.path.clone(),
                line: 1,
                column: 1,
                message: "missing leading `--!strict` directive".to_string(),
            });
        }
    }

    // Agent-name lint against the merged registry (name presence; values
    // interpolate at resolve time, names do not).
    let known: HashSet<String> = cfg.registry.agent_names().into_iter().collect();
    for file in &walked.parsed {
        for call in &file.agents {
            if !known.contains(&call.name) {
                findings.push(Finding {
                    path: file.path.clone(),
                    line: call.site.line,
                    column: call.site.column,
                    message: format!(
                        "unknown agent `{}`: not found in the user or project registry",
                        call.name
                    ),
                });
            }
        }
    }

    // Interaction lint: with ask call sites present, a prohibited or
    // unresolvable posture is a finding; with none, never.
    findings.extend(interaction_findings(&walked.parsed, &cfg.interaction));

    for finding in &findings {
        eprintln!("{}", finding.render(cfg.color));
    }
    if !findings.is_empty() {
        eprintln!("{}", summary_line(&findings));
    }

    // Typecheck pass: run even when in-process findings exist, so the
    // author sees everything in one invocation. luau-lsp's stderr passes
    // through unmodified; only its exit status is load-bearing here.
    let Some(lsp) = find_luau_lsp() else {
        eprintln!(
            "error: luau-lsp not found on PATH; the typecheck pass requires it \
             (https://github.com/luau-lsp/luau-lsp)"
        );
        return 2;
    };
    match run_luau_lsp(&lsp, &entry) {
        Ok(status) if status.success() => {}
        Ok(status) => {
            eprintln!("luau-lsp analyze exited with {status}");
            return 1;
        }
        Err(e) => {
            eprintln!("error: failed to run luau-lsp: {e}");
            return 2;
        }
    }

    u8::from(!findings.is_empty())
}

/// The `run` pre-flight: compile + require + agent + interaction lints
/// over the entry and its literal require graph. No strictness
/// enforcement, no luau-lsp, no execution. An empty result means the
/// run may proceed; anything else fails the run (exit 1) before any
/// agent subprocess spawns.
///
/// Reading the entry is left to `run` itself; if it fails here the
/// findings are simply empty.
pub fn preflight(
    script: &Path,
    registry: &Registry,
    interaction: &InteractionMode,
) -> Vec<Finding> {
    let entry = std::fs::canonicalize(script).unwrap_or_else(|_| script.to_path_buf());
    let Ok(source) = std::fs::read_to_string(&entry) else {
        return Vec::new();
    };

    let walked = lint::walk(&entry);
    let mut files = vec![(entry, source)];
    for file in &walked.parsed {
        if let Ok(src) = std::fs::read_to_string(&file.path) {
            files.push((file.path.clone(), src));
        }
    }
    let mut findings = compile_findings(&files);
    findings.extend(walked.failures);
    findings.extend(walked.broken);

    let known: HashSet<String> = registry.agent_names().into_iter().collect();
    for file in &walked.parsed {
        for call in &file.agents {
            if !known.contains(&call.name) {
                findings.push(Finding {
                    path: file.path.clone(),
                    line: call.site.line,
                    column: call.site.column,
                    message: format!(
                        "unknown agent `{}`: not found in the user or project registry",
                        call.name
                    ),
                });
            }
        }
    }
    findings.extend(interaction_findings(&walked.parsed, interaction));
    findings
}

/// Whether `path` sits under a `luau_packages` directory (ptah's
/// generated package artifacts — exempt from the strict-directive
/// lint, which applies to authored files).
fn under_luau_packages(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "luau_packages")
}

/// Interaction findings for the ask call sites in the walked graph:
/// one finding per `ptah.ask(` site when the resolved posture is
/// `Prohibited` (the deliberate `none` contradicting the script's
/// need) or `Unresolved` (nothing selected a provider and
/// auto-detection could not — the CI case), none when a provider
/// resolved or the reachable set contains no ask sites. Over-approximate
/// by design (an ask on a never-executed branch still fails under
/// `none`); the runtime check remains the source of truth.
fn interaction_findings(parsed: &[ParsedFile], mode: &InteractionMode) -> Vec<Finding> {
    match mode {
        InteractionMode::Provider(_) => Vec::new(),
        InteractionMode::Prohibited | InteractionMode::Unresolved => {
            let (kind, remedy) = match mode {
                InteractionMode::Prohibited => (
                    "interaction is prohibited (`none`)",
                    "allow a provider (--ask=stdin, PTAH_ASK, or [ask]) or remove the ask",
                ),
                // Named remedies are the contract (cli/check specs).
                _ => (
                    "no ask provider resolvable",
                    "pass --ask, set PTAH_ASK, or configure [ask] (auto-detection \
                     needs a terminal on stdin and stdout)",
                ),
            };
            parsed
                .iter()
                .flat_map(|f| {
                    f.asks.iter().map(|ask| Finding {
                        path: f.path.clone(),
                        line: ask.site.line,
                        column: ask.site.column,
                        message: format!(
                            "`ptah.ask` is unusable: {kind} — {remedy}"
                        ),
                    })
                })
                .collect()
        }
    }
}

/// Compile each `(path, source)` with a fresh sandboxed Luau instance —
/// [`mlua::Chunk::into_function`] compiles the chunk without ever
/// calling it. Syntax failures become positioned findings (the line is
/// parsed from the compiler's `name:line: message` diagnostic; it does
/// not report columns).
fn compile_findings(files: &[(PathBuf, String)]) -> Vec<Finding> {
    let lua = match mlua::Lua::new_with(mlua::StdLib::TABLE, mlua::LuaOptions::default()) {
        Ok(lua) => lua,
        Err(e) => {
            return files
                .iter()
                .map(|(path, _)| Finding {
                    path: path.clone(),
                    line: 1,
                    column: 1,
                    message: format!("failed to initialize compile environment: {e}"),
                })
                .collect();
        }
    };
    let _ = lua.sandbox(true);

    let mut findings = Vec::new();
    for (path, source) in files {
        if let Err(e) = lua
            .load(source.as_str())
            .set_name(format!("@{}", path.display()))
            .into_function()
        {
            let (line, message) = match &e {
                mlua::Error::SyntaxError { message, .. } => {
                    let (line, rest) = split_line_prefix(message, path);
                    (line, format!("syntax error: {rest}"))
                }
                other => (1, format!("compile error: {other}")),
            };
            findings.push(Finding {
                path: path.clone(),
                line,
                column: 1,
                message,
            });
        }
    }
    findings
}

/// Split a compiler diagnostic of the form `<path>:<line>: <message>` into
/// `(line, message)`, falling back to `(1, whole)` when it doesn't match.
fn split_line_prefix(message: &str, path: &Path) -> (u32, String) {
    let rest = message
        .strip_prefix(&format!("{}:", path.display()))
        .unwrap_or(message);
    match rest.split_once(": ") {
        Some((line, msg)) => (line.parse().unwrap_or(1), msg.to_string()),
        None => (1, rest.to_string()),
    }
}

/// Locate an executable `name` on PATH (`which`-style scan; gives the
/// clean "install luau-lsp" error instead of a spawn failure).
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .find_map(|dir| {
            #[cfg(unix)]
            {
                let candidate = dir.join(name);
                is_executable(&candidate).then_some(candidate)
            }
            #[cfg(windows)]
            {
                ["luau-lsp.exe", "luau-lsp.cmd", "luau-lsp.bat", name]
                    .iter()
                    .map(|n| dir.join(n))
                    .find_map(|c| c.is_file().then_some(c))
            }
        })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file() && std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

fn find_luau_lsp() -> Option<PathBuf> {
    find_on_path("luau-lsp")
}

/// Run `luau-lsp analyze` over the entry with the binary's embedded
/// definitions written to a unique temp file. Stderr passes through raw
/// (inherited); stdout is discarded so it cannot carry findings.
fn run_luau_lsp(bin: &Path, entry: &Path) -> std::io::Result<std::process::ExitStatus> {
    let defs = write_definitions_temp()?;
    let status = Command::new(bin)
        .arg("analyze")
        .arg("--platform=standard")
        .arg(format!("--definitions={}", defs.display()))
        .arg(entry)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status();
    // Best-effort cleanup; content is not secret.
    let _ = std::fs::remove_file(&defs);
    status
}

/// Write the embedded type definitions to a unique temp file so the
/// check always typechecks against the installed binary's API version.
fn write_definitions_temp() -> std::io::Result<PathBuf> {
    let unique = format!(
        "ptah-defs-{}-{}.luau",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::write(&path, defs::TYPE_DEFINITIONS)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding_at(line: u32, column: u32) -> Finding {
        Finding {
            path: PathBuf::from("/tmp/s.luau"),
            line,
            column,
            message: "boom".to_string(),
        }
    }

    #[test]
    fn findings_render_with_and_without_color() {
        let f = finding_at(3, 7);
        assert_eq!(f.render(false), "/tmp/s.luau:3:7: boom");
        let colored = f.render(true);
        assert!(
            colored.starts_with("\x1b[1;31m/tmp/s.luau:3:7:\x1b[0m boom"),
            "{colored}"
        );
    }

    #[test]
    fn summary_counts_findings_and_files() {
        let mut f1 = finding_at(1, 1);
        f1.path = PathBuf::from("/tmp/a.luau");
        let mut f2 = finding_at(2, 1);
        f2.path = PathBuf::from("/tmp/a.luau");
        let mut f3 = finding_at(1, 1);
        f3.path = PathBuf::from("/tmp/b.luau");
        assert_eq!(summary_line(&[f1.clone()]), "found 1 finding in 1 file");
        assert_eq!(summary_line(&[f1, f2, f3]), "found 3 findings in 2 files");
    }

    #[test]
    fn compile_pass_reports_positioned_syntax_errors() {
        let dir = std::env::temp_dir().join(format!("ptah-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("broken.luau");
        std::fs::write(&path, "local x = {\n").unwrap();
        let findings = compile_findings(&[(path.clone(), "local x = {\n".to_string())]);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].path, path);
        assert!(findings[0].line >= 1);
        assert!(
            findings[0].message.starts_with("syntax error:"),
            "{:?}",
            findings[0].message
        );
        // Clean source compiles to no findings.
        assert!(compile_findings(&[(path, "return 1".to_string())]).is_empty());
    }

    #[test]
    fn line_prefix_splitter_parses_compiler_diagnostics() {
        let path = Path::new("/tmp/s.luau");
        let (line, msg) = split_line_prefix("/tmp/s.luau:3: Expected '}'", path);
        assert_eq!((line, msg.as_str()), (3, "Expected '}'"));
        // Fallbacks: no prefix match, no line.
        let (line, msg) = split_line_prefix("totally different", path);
        assert_eq!((line, msg.as_str()), (1, "totally different"));
        let (line, _) = split_line_prefix("/tmp/s.luau:zz: x", path);
        assert_eq!(line, 1);
    }
}
