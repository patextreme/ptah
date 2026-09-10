//! The full-moon lint walk: parse the entry and every file reachable
//! through literal `require("...")` string arguments — never executing
//! anything — collecting literal `ptah.agent("...")` call sites, broken
//! requires, parse failures, and leading `--!strict` directives.
//!
//! Matching policy (settled in the change design): only *literal* call
//! shapes are linted — `require("<string>")` / `require "<string>"` where
//! the callee is the global name `require`, and `ptah.agent("<string>")`
//! where the callee is literally the global member access. Computed
//! arguments, aliased references (`local a = ptah.agent`), and
//! commented-out calls are not linted, so the walk cannot produce false
//! findings about code the runtime would resolve differently.

use std::collections::{HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};

use full_moon::ast::{Call, Expression, FunctionArgs, FunctionCall, Index, Prefix, Suffix};
use full_moon::tokenizer::{Position, Token, TokenKind, TokenType};
use full_moon::visitors::Visitor;

use super::Finding;

/// A literal call site position (1-based line/column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CallSite {
    pub line: u32,
    pub column: u32,
}

/// A literal `require("<module>")` call found while walking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequireCall {
    pub site: CallSite,
    pub module: String,
}

/// A literal `ptah.agent("<name>")` call found while walking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentCall {
    pub site: CallSite,
    pub name: String,
}

/// A `ptah.ask(...)` call found while walking — any argument form; the
/// call site itself is the capability signal (unlike agent names, no
/// literal restriction applies). Alias-indirected asks
/// (`local f = ptah.ask`) are not calls and are not collected (a
/// documented residual; the runtime check is the source of truth).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AskCall {
    pub site: CallSite,
}

/// Facts about one successfully parsed file in the require graph.
#[derive(Debug, Clone)]
pub(crate) struct ParsedFile {
    pub path: PathBuf,
    pub agents: Vec<AgentCall>,
    /// `ptah.ask(` call sites in this file (any argument form).
    pub asks: Vec<AskCall>,
    /// The file begins with a `--!strict` hot-comment.
    pub strict: bool,
}

/// The result of walking the entry's literal require graph.
#[derive(Debug, Default)]
pub(crate) struct WalkResult {
    /// Successfully parsed files: the entry first, then required files in
    /// discovery order (each file once, even under require cycles).
    pub parsed: Vec<ParsedFile>,
    /// Broken literal requires: non-relative strings and targets with
    /// no module file.
    pub broken: Vec<Finding>,
    /// Files that could not be read or parsed (`path:line:col: message`).
    pub failures: Vec<Finding>,
}

/// Lexically normalize a path: resolve `.`/`..` components without
/// touching the filesystem (`..` at the root pops). Same directory rules
/// as the runtime navigator (`script::require`), duplicated on purpose:
/// the lint walk is zero-execution by construction and must not depend on
/// the script host.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// A literal require string is navigable only when explicitly relative
/// or an alias form.
fn is_relative_module(module: &str) -> bool {
    module.starts_with("./") || module.starts_with("../")
}

fn is_alias_module(module: &str) -> bool {
    module.starts_with('@')
}

/// One discovered `.luaurc` alias configuration: the file's directory
/// and its parsed `aliases` table.
#[derive(Debug, Clone)]
struct AliasConfig {
    /// Directory containing the `.luaurc` (alias targets anchor here).
    dir: PathBuf,
    /// alias name -> target path string, exactly as written.
    aliases: std::collections::BTreeMap<String, String>,
}

impl AliasConfig {
    /// Find the nearest `.luaurc` at or above `dir` whose `aliases`
    /// table defines `alias` — Luau's navigator keeps walking past
    /// configurations that parse but lack the alias. A
    /// `.config.luau` (Luau-source configuration) cannot be parsed
    /// statically — documented divergence: ptah never writes one,
    /// and the runtime remains the source of truth for it.
    fn discover(dir: &Path, alias: &str) -> Option<Self> {
        let mut current: &Path = dir;
        loop {
            let candidate = current.join(".luaurc");
            if candidate.is_file()
                && let Ok(text) = std::fs::read_to_string(&candidate)
                && let Some(aliases) = parse_aliases(&text)
                && aliases.contains_key(alias)
            {
                return Some(Self {
                    dir: current.to_path_buf(),
                    aliases,
                });
            }
            current = current.parent()?;
        }
    }
}

/// Extract the `aliases` table from `.luaurc` JSON (JSONC: comments
/// stripped first). Returns `None` when the file has no `aliases`
/// table or fails to parse — the caller treats that as "no alias
/// configuration here" and the require becomes a finding naming the
/// alias.
fn parse_aliases(text: &str) -> Option<std::collections::BTreeMap<String, String>> {
    let stripped: String = strip_jsonc_comments(text);
    let value: serde_json::Value = serde_json::from_str(&stripped).ok()?;
    let aliases = value.get("aliases")?.as_object()?;
    Some(
        aliases
            .iter()
            .filter_map(|(k, v)| {
                v.as_str().map(|target| (k.clone(), target.to_string()))
            })
            .collect(),
    )
}

/// Strip `//`-line and `/*`-block comments outside strings — enough for
/// `.luaurc` JSONC as written by editors and tools.
fn strip_jsonc_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(&escaped) = chars.peek() {
                    out.push(escaped);
                    chars.next();
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut closed = false;
                while let Some(c2) = chars.next() {
                    if c2 == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        closed = true;
                        break;
                    }
                    if c2 == '\n' {
                        out.push('\n');
                    }
                }
                let _ = closed; // unterminated comment: treated as ended
            }
            other => out.push(other),
        }
    }
    out
}

/// Resolve an alias require's target the way the runtime does: nearest
/// `.luaurc` at or above the requiring file's directory, alias target
/// anchored at that config's directory, remaining segments appended.
fn resolve_alias(from_dir: &Path, alias_path: &str) -> Result<PathBuf, String> {
    let (alias, rest) = match alias_path[1..].split_once('/') {
        Some((a, r)) => (a, Some(r)),
        None => (&alias_path[1..], None),
    };
    // Luau lowercases alias names during lookup.
    let alias = alias.to_ascii_lowercase();
    let config = AliasConfig::discover(from_dir, &alias).ok_or_else(|| {
        format!(
            "cannot resolve alias `{alias}`: no .luaurc defining it found above {}",
            from_dir.display()
        )
    })?;
    let target = config.aliases.get(&alias).ok_or_else(|| {
        format!("cannot resolve alias `{alias}`: not defined in {}", config.dir.join(".luaurc").display())
    })?;
    // Alias targets anchor like relative paths at the config's
    // directory (the runtime resolves `./`-form targets there).
    let mut joined = config.dir.join(target.trim_start_matches("./"));
    if let Some(rest) = rest {
        joined = joined.join(rest);
    }
    Ok(normalize(&joined))
}

/// Resolve an already-joined module path to a physical file:
/// `<p>.luau`, `<p>.lua`, `<p>/init.luau`, `<p>/init.lua`.
fn resolve_file(path: &Path) -> Option<PathBuf> {
    for ext in ["luau", "lua"] {
        let candidate = path.with_extension(ext);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    for init in ["init.luau", "init.lua"] {
        let candidate = path.join(init);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Walk the literal require graph from `entry` (a canonicalized path),
/// resolving every literal require edge with the same pure rules the
/// runtime navigator uses.
pub(crate) fn walk(entry: &Path) -> WalkResult {
    let mut result = WalkResult::default();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::from([entry.to_path_buf()]);

    while let Some(path) = queue.pop_front() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                result.failures.push(Finding {
                    path,
                    line: 1,
                    column: 1,
                    message: format!("cannot read file: {e}"),
                });
                continue;
            }
        };
        let ast = match full_moon::parse(&source) {
            Ok(ast) => ast,
            Err(errors) => {
                let (line, column, message) = match errors.first() {
                    Some(err) => {
                        let pos = err.range().0;
                        (
                            pos.line().max(1) as u32,
                            pos.character().max(1) as u32,
                            err.error_message().to_string(),
                        )
                    }
                    None => (1, 1, "parse error".to_string()),
                };
                result.failures.push(Finding {
                    path,
                    line,
                    column,
                    message: format!("parse error: {message}"),
                });
                continue;
            }
        };

        let mut collector = Collector::default();
        collector.visit_ast(&ast);

        for req in &collector.requires {
            match resolve_edge(&path, &req.module) {
                Ok(target) => queue.push_back(target),
                Err(message) => result.broken.push(Finding {
                    path: path.clone(),
                    line: req.site.line,
                    column: req.site.column,
                    message,
                }),
            }
        }

        result.parsed.push(ParsedFile {
            path,
            agents: collector.agents,
            asks: collector.asks,
            strict: collector.strict,
        });
    }

    result
}

/// Resolve one literal require edge exactly as the runtime would: the
/// physical module file, or a finding message naming the problem.
fn resolve_edge(from_file: &Path, module: &str) -> Result<PathBuf, String> {
    let from_dir = from_file.parent().unwrap_or(Path::new("."));
    if is_alias_module(module) {
        let target = resolve_alias(from_dir, module)?;
        return resolve_file(&target).ok_or_else(|| {
            format!(
                "cannot resolve require `{module}`: no module file at {}",
                target.display()
            )
        });
    }
    if !is_relative_module(module) {
        return Err(format!(
            "require path is not relative to the script: `{module}` \
             (only \"./\", \"../\", and \"@alias\" paths are allowed)"
        ));
    }
    let target = normalize(&from_dir.join(module));
    resolve_file(&target).ok_or_else(|| {
        format!(
            "cannot resolve require `{module}`: no module file at {}",
            target.display()
        )
    })
}

/// Extract the string literal and position of a single-literal call
/// argument. Only plain (escape-free) literals are used: strings with
/// backslash escapes are left to the runtime rather than interpreted
/// here (no false findings).
fn literal_arg(args: &FunctionArgs) -> Option<(String, Position)> {
    match args {
        FunctionArgs::Parentheses { arguments, .. } => {
            if arguments.len() != 1 {
                return None;
            }
            match arguments.iter().next().unwrap() {
                Expression::String(token) => literal_string(token),
                // Parenthesized/computed/binary expressions are not
                // literal-only matches.
                _ => None,
            }
        }
        // `require "./x"` call-string form.
        FunctionArgs::String(token) => literal_string(token),
        // Table-call args (`f{...}`) are never literal; anything a newer
        // grammar adds is not either.
        _ => None,
    }
}

fn literal_string(token: &full_moon::tokenizer::TokenReference) -> Option<(String, Position)> {
    match token.token().token_type() {
        TokenType::StringLiteral { literal, .. } => {
            let s = literal.to_string();
            if s.contains('\\') {
                None
            } else {
                Some((s, token.token().start_position()))
            }
        }
        _ => None,
    }
}

fn site(pos: Position) -> CallSite {
    CallSite {
        line: pos.line().max(1) as u32,
        column: pos.character().max(1) as u32,
    }
}

/// One pass over a file's AST: literal call collection plus leading
/// hot-comment detection.
#[derive(Default)]
struct Collector {
    requires: Vec<RequireCall>,
    agents: Vec<AgentCall>,
    asks: Vec<AskCall>,
    /// Byte offset of the first non-trivia token seen (the hot-comment
    /// region ends there, matching how Luau and luau-lsp read hot
    /// comments). Full-moon visits a token reference's inner token
    /// before its leading trivia, so ordering is decided by position,
    /// not visit order.
    first_code_bytes: Option<usize>,
    strict: bool,
}

impl Visitor for Collector {
    fn visit_function_call(&mut self, call: &FunctionCall) {
        let Prefix::Name(prefix) = call.prefix() else {
            return;
        };
        let name = prefix.token().to_string();
        let suffixes: Vec<&Suffix> = call.suffixes().collect();

        // require("./x") / require "./x"
        if name == "require" && suffixes.len() == 1 {
            if let Suffix::Call(Call::AnonymousCall(args)) = suffixes[0]
                && let Some((module, pos)) = literal_arg(args)
            {
                self.requires.push(RequireCall {
                    site: site(pos),
                    module,
                });
            }
            return;
        }

        // ptah.agent("name")
        if name == "ptah"
            && suffixes.len() == 2
            && let Suffix::Index(Index::Dot { name: member, .. }) = suffixes[0]
            && member.token().to_string() == "agent"
            && let Suffix::Call(Call::AnonymousCall(args)) = suffixes[1]
            && let Some((agent_name, _)) = literal_arg(args)
        {
            // Position the finding at the call start (`ptah`), not the
            // argument: the whole call is the problem when the name is
            // unknown.
            self.agents.push(AgentCall {
                site: site(prefix.token().start_position()),
                name: agent_name,
            });
        }

        // ptah.ask(...) — any argument form (table, string, computed,
        // none): the call itself is the interaction signal. Same shape
        // as the agent branch minus the literal restriction.
        if name == "ptah"
            && suffixes.len() == 2
            && let Suffix::Index(Index::Dot { name: member, .. }) = suffixes[0]
            && member.token().to_string() == "ask"
            && matches!(suffixes[1], Suffix::Call(Call::AnonymousCall(_)))
        {
            self.asks.push(AskCall {
                site: site(prefix.token().start_position()),
            });
        }
    }

    fn visit_token(&mut self, token: &Token) {
        let bytes = token.start_position().bytes();
        match token.token_kind() {
            TokenKind::SingleLineComment => {
                if let TokenType::SingleLineComment { comment, .. } = token.token_type()
                    && comment.trim() == "!strict"
                    && bytes < self.first_code_bytes.unwrap_or(usize::MAX)
                {
                    self.strict = true;
                }
            }
            // Block comments, whitespace, and a shebang line may precede
            // the hot comment without disqualifying it.
            TokenKind::MultiLineComment | TokenKind::Whitespace | TokenKind::Shebang => {}
            _ => {
                self.first_code_bytes =
                    Some(self.first_code_bytes.unwrap_or(usize::MAX).min(bytes));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_project(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ptah-lint-{}-{name}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("lib")).unwrap();
        fs::write(dir.join("lib/util.luau"), "--!strict\nreturn {}\n").unwrap();
        dir
    }

    fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
        let p = dir.join(rel);
        fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn collects_literal_calls_and_ignores_others() {
        let dir = tmp_project("collect");
        let entry = write(
            &dir,
            "main.luau",
            "--!strict\n\
             local a = require(\"./lib/util\")\n\
             local b = require \"./lib/util\"\n\
             local agent = ptah.agent(\"claude\")\n\
             local name = \"computed\"\n\
             local c = require(name)\n\
             local d = require(\"./lib/\" .. name)\n\
             local alias = ptah.agent\n\
             local e = alias(\"ghost\")\n\
             local f = ptah.agent(name)\n\
             local g = ptah.agent({ command = \"x\" })\n\
             -- local h = require(\"./missing/commented\")\n\
             local h = ptah.spawn(function() end)\n\
             print(a, b, c, d, e, f, g, agent, alias, h)\n",
        );
        let walk_result = walk(&entry);
        assert!(walk_result.broken.is_empty(), "{:?}", walk_result.broken);
        assert!(
            walk_result.failures.is_empty(),
            "{:?}",
            walk_result.failures
        );
        assert_eq!(walk_result.parsed.len(), 2, "{:?}", walk_result.parsed);
        let main = &walk_result.parsed[0];
        assert!(main.strict);
        // Both literal require forms (`require("...")` and `require
        // "..."`) resolved to the same module — exactly one file beyond
        // the entry; computed/aliased forms contributed no edges.
        assert!(
            walk_result.parsed[1].path.ends_with("lib/util.luau"),
            "{:?}",
            walk_result.parsed
        );
        // Exactly one literal agent call: computed, aliased, table, and
        // commented-out forms are ignored.
        assert_eq!(main.agents.len(), 1);
        assert_eq!(main.agents[0].name, "claude");
        assert_eq!(main.agents[0].site.line, 4);
        // No ask calls in this fixture.
        assert!(main.asks.is_empty());
    }

    #[test]
    fn collects_ask_calls_any_argument_form_and_ignores_others() {
        let dir = tmp_project("ask-collect");
        let entry = write(
            &dir,
            "main.luau",
            "--!strict\n\
             local a = ptah.ask({ prompt = \"q\" })\n\
             local b = ptah.ask(\"computed \" .. \"arg\")\n\
             local c = ptah.ask()\n\
             local d = ptah.ask { prompt = \"table-call form\" }\n\
             local e = ptah.other({ prompt = \"q\" })\n\
             local ask = ptah.ask\n\
             local f = ask({ prompt = \"aliased\" })\n\
             -- ptah.ask({ prompt = \"commented\" })\n\
             local g = ptah.askx({ prompt = \"prefix collision\" })\n\
             print(a, b, c, d, e, f, g, ask)\n",
        );
        let walked = walk(&entry);
        assert_eq!(walked.parsed.len(), 1);
        let main = &walked.parsed[0];
        // Four collected: literal table, computed string argument, no
        // args, and the table-call form — any argument form counts,
        // the call itself is the signal.
        assert_eq!(main.asks.len(), 4, "{:?}", main.asks);
        assert_eq!(main.asks[0].site.line, 2);
        // Not collected: `ptah.other`, the aliased indirect call (a
        // documented residual), commented-out code, and the
        // `ptah.askx` prefix collision.
        for line in [6, 7, 8, 9] {
            assert!(
                !main.asks.iter().any(|a| a.site.line == line),
                "line {line} must not be collected: {:?}",
                main.asks
            );
        }
    }

    #[test]
    fn strict_directive_variants() {
        let dir = tmp_project("strict");
        let cases = [
            ("--!strict\nreturn 1\n", true),
            ("--!strict", true), // directive alone
            ("-- other comment\n--!strict\nreturn 1\n", true),
            ("--!strict\n", true),
            ("--! strict\nreturn 1\n", false), // space breaks the hot comment
            ("return 1\n--!strict\n", false),  // not leading
            ("--!nonstrict\nreturn 1\n", false),
            ("--[[]]\n--!strict\nreturn 1\n", true), // block comment before is fine
        ];
        for (i, (body, expect)) in cases.iter().enumerate() {
            let p = write(&dir, &format!("s{i}.luau"), body);
            let ast = full_moon::parse(body).unwrap();
            let mut collector = Collector::default();
            collector.visit_ast(&ast);
            assert_eq!(collector.strict, *expect, "case {i}: {body:?} ({p:?})");
        }
    }

    #[test]
    fn walks_graph_cycles_and_positions() {
        let dir = tmp_project("graph");
        write(
            &dir,
            "main.luau",
            "--!strict\nlocal b = require(\"./lib/b\")\nreturn b\n",
        );
        write(
            &dir,
            "lib/b.luau",
            "--!strict\nlocal main = require(\"../main\")\nreturn main\n",
        );
        let entry = dir.join("main.luau");
        let walk_result = walk(&entry);
        assert_eq!(walk_result.parsed.len(), 2, "cycle must terminate");
        assert!(walk_result.failures.is_empty() && walk_result.broken.is_empty());
        assert!(walk_result.parsed.iter().all(|f| f.strict));
    }

    #[test]
    fn broken_requires_reported_with_positions() {
        let dir = tmp_project("broken");
        let entry = write(
            &dir,
            "main.luau",
            "--!strict\n\
             local a = require(\"./lib/missing\")\n\
             local c = require(\"shared/helper\")\n\
             return a, c\n",
        );
        let walk_result = walk(&entry);
        assert_eq!(walk_result.broken.len(), 2, "{:?}", walk_result.broken);
        // All findings point at the requiring file with the call's line.
        assert!(
            walk_result
                .broken
                .iter()
                .all(|f| f.path == entry && f.line >= 2 && f.column >= 1)
        );
        assert!(
            walk_result.broken[0]
                .message
                .contains("cannot resolve require `./lib/missing`")
        );
        assert!(
            walk_result.broken[1]
                .message
                .contains("not relative to the script")
        );
    }

    // ------------------------------------------------------------------
    // Alias requires (resolved through the nearest .luaurc, exactly
    // like the runtime).
    // ------------------------------------------------------------------

    fn alias_project(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "ptah-lint-alias-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join(".ptah/workflows")).unwrap();
        fs::create_dir_all(base.join(".ptah/luau_packages")).unwrap();
        fs::write(
            base.join(".luaurc"),
            r#"{ "aliases": { "hello": "./.ptah/luau_packages/hello" } }"#,
        )
        .unwrap();
        fs::write(
            base.join(".ptah/luau_packages/hello.luau"),
            "--!strict\nreturn {}\n",
        )
        .unwrap();
        base
    }

    #[test]
    fn alias_require_over_an_installed_package_is_not_a_finding() {
        let base = alias_project("ok");
        let entry = write(
            &base.join(".ptah/workflows"),
            "main.luau",
            "--!strict\nlocal h = require(\"@hello\")\nreturn h\n",
        );
        let walked = walk(&entry);
        assert!(walked.broken.is_empty(), "{:?}", walked.broken);
        assert_eq!(walked.parsed.len(), 2, "the package module is walked");
        // Deep alias paths and the package's own relative requires
        // resolve too.
        fs::create_dir_all(base.join(".ptah/luau_packages/hello/sub")).unwrap();
        fs::write(
            base.join(".ptah/luau_packages/hello/sub/init.luau"),
            "--!strict\nreturn {}\n",
        )
        .unwrap();
        let entry = write(
            &base.join(".ptah/workflows"),
            "deep.luau",
            "--!strict\nlocal s = require(\"@hello/sub\")\nreturn s\n",
        );
        let walked = walk(&entry);
        assert!(walked.broken.is_empty(), "{:?}", walked.broken);
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn undefined_alias_is_a_finding_naming_the_alias() {
        let base = alias_project("undefined");
        let entry = write(
            &base.join(".ptah/workflows"),
            "main.luau",
            "--!strict\nlocal x = require(\"@nope\")\nreturn x\n",
        );
        let walked = walk(&entry);
        assert_eq!(walked.broken.len(), 1, "{:?}", walked.broken);
        assert!(
            walked.broken[0].message.contains("`nope`"),
            "names the alias: {}",
            walked.broken[0].message
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn alias_targeting_a_missing_module_is_a_finding() {
        let base = alias_project("missing-target");
        fs::remove_file(base.join(".ptah/luau_packages/hello.luau")).unwrap();
        let entry = write(
            &base.join(".ptah/workflows"),
            "main.luau",
            "--!strict\nlocal x = require(\"@hello\")\nreturn x\n",
        );
        let walked = walk(&entry);
        assert_eq!(walked.broken.len(), 1, "{:?}", walked.broken);
        assert!(
            walked.broken[0].message.contains("@hello"),
            "{}",
            walked.broken[0].message
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn alias_without_any_configuration_is_a_finding() {
        let base = alias_project("no-config");
        fs::remove_file(base.join(".luaurc")).unwrap();
        let entry = write(
            &base.join(".ptah/workflows"),
            "main.luau",
            "--!strict\nlocal x = require(\"@hello\")\nreturn x\n",
        );
        let walked = walk(&entry);
        assert_eq!(walked.broken.len(), 1, "{:?}", walked.broken);
        assert!(
            walked.broken[0].message.contains("no .luaurc"),
            "{}",
            walked.broken[0].message
        );
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn nearest_luaurc_defining_the_alias_wins() {
        let base = alias_project("nearest");
        // A nearer config without the alias: Luau's navigator keeps
        // walking up, so the project root's `hello` still resolves
        // (runtime semantics — the search is for the alias, not for
        // any config).
        fs::write(
            base.join(".ptah/workflows/.luaurc"),
            "{ \"languageMode\": \"strict\" }",
        )
        .unwrap();
        let entry = write(
            &base.join(".ptah/workflows"),
            "main.luau",
            "--!strict\nlocal x = require(\"@hello\")\nreturn x\n",
        );
        let walked = walk(&entry);
        assert!(walked.broken.is_empty(), "{:?}", walked.broken);

        // But a nearer config that *defines* the alias shadows the
        // root's (and this one points at a missing target).
        fs::write(
            base.join(".ptah/workflows/.luaurc"),
            "{ \"aliases\": { \"hello\": \"./nope\" } }",
        )
        .unwrap();
        let walked = walk(&entry);
        assert_eq!(walked.broken.len(), 1, "{:?}", walked.broken);
        assert!(walked.broken[0].message.contains("@hello"));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn jsonc_comments_do_not_break_alias_parsing() {
        let text = "{\n  // project aliases\n  \"aliases\": {\n    /* hello */\n    \"hello\": \"./pkg/hello\"\n  }\n}\n";
        let aliases = parse_aliases(text).unwrap();
        assert_eq!(aliases["hello"], "./pkg/hello");
    }

    #[test]
    fn cross_tree_require_is_not_a_finding() {
        // A require target outside the entry's directory is walked and
        // linted like any other module: no escape finding exists.
        let base = std::env::temp_dir().join(format!("ptah-lint-cross-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("workflow")).unwrap();
        fs::create_dir_all(base.join("shared")).unwrap();
        fs::write(
            base.join("workflow/main.luau"),
            "--!strict\nlocal h = require(\"../shared/helper\")\nreturn h\n",
        )
        .unwrap();
        fs::write(base.join("shared/helper.luau"), "--!strict\nreturn {}\n").unwrap();
        let walk_result = walk(&base.join("workflow/main.luau"));
        assert!(walk_result.broken.is_empty(), "{:?}", walk_result.broken);
        assert!(
            walk_result.failures.is_empty(),
            "{:?}",
            walk_result.failures
        );
        assert_eq!(walk_result.parsed.len(), 2, "{:?}", walk_result.parsed);
        assert!(walk_result.parsed.iter().all(|f| f.strict));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn parse_failure_becomes_positioned_finding() {
        let dir = tmp_project("parse");
        let entry = write(&dir, "main.luau", "--!strict\nlocal x = {\n");
        let walk_result = walk(&entry);
        assert!(walk_result.parsed.is_empty());
        assert_eq!(walk_result.failures.len(), 1);
        let f = &walk_result.failures[0];
        assert_eq!(f.path, entry);
        assert!(f.line >= 1 && f.column >= 1);
        assert!(f.message.starts_with("parse error:"), "{}", f.message);
    }

    #[test]
    fn escaped_string_literals_are_not_interpreted() {
        let dir = tmp_project("backslash");
        // A backslash in the literal means the runtime value differs from
        // the source text: skip, no false findings.
        let entry = write(
            &dir,
            "main.luau",
            "--!strict\nlocal a = require(\"./lib\\tweird\")\nreturn a\n",
        );
        let walk_result = walk(&entry);
        assert!(walk_result.broken.is_empty(), "{:?}", walk_result.broken);
    }
}
