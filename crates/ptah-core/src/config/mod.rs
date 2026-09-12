//! Agent registry configuration model: agent launch specs, registry
//! merging (project-wins precedence), and `${VAR}` interpolation.
//!
//! Registries map agent names to launch specs:
//!
//! ```toml
//! [agents.claude]
//! command = "npx"
//! args = ["-y", "@agentclientprotocol/claude-agent-acp"]
//! env = { ANTHROPIC_MODEL = "${MODEL}" }
//! ```
//!
//! Values are stored raw; interpolation happens at resolve time, against
//! ptah's process environment. Project entries override user entries
//! wholesale per agent name (TOML parsing and file discovery live in
//! `config_fs`, not here).

use std::collections::BTreeMap;

/// A single agent launch specification, as written in a registry file.
///
/// Values are stored raw; `${VAR}` interpolation happens at resolve time.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct AgentSpec {
    /// Program to spawn.
    pub command: String,
    /// Extra arguments for the program.
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment overrides merged over the inherited environment.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl AgentSpec {
    /// A spec with just a command.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }
    }

    /// Interpolate `${VAR}` (unset → empty) across command, args, and env values.
    pub fn interpolate(&self, lookup: &dyn Fn(&str) -> Option<String>) -> AgentSpec {
        AgentSpec {
            command: interpolate(&self.command, lookup),
            args: self.args.iter().map(|a| interpolate(a, lookup)).collect(),
            env: self
                .env
                .iter()
                .map(|(k, v)| (k.clone(), interpolate(v, lookup)))
                .collect(),
        }
    }
}

/// Replace every `${VAR}` occurrence in `s` with the lookup result
/// (empty string when the variable is unset).
fn interpolate(s: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let var = &after[..end];
                if let Some(v) = lookup(var) {
                    out.push_str(&v);
                }
                rest = &after[end + 1..];
            }
            None => {
                // Unterminated: keep literally.
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// The known ask providers — the closed value set behind `--ask`,
/// `PTAH_ASK`, and the registry's `[ask] provider` key (parsed once,
/// consumed everywhere, so the three readers cannot drift).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskProviderKind {
    /// Prompts render on stdout; answers read from stdin (works over
    /// pipes whenever explicitly selected).
    Stdin,
    /// Asking prohibited outright (`ptah.ask` raises; the headless
    /// permission posture is unaffected).
    None,
}

impl std::str::FromStr for AskProviderKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stdin" => Ok(Self::Stdin),
            "none" => Ok(Self::None),
            other => Err(format!(
                "unknown ask provider `{other}` (accepted values: `stdin`, `none`)"
            )),
        }
    }
}

impl std::fmt::Display for AskProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AskProviderKind::Stdin => "stdin",
            AskProviderKind::None => "none",
        })
    }
}

/// The registry's first global (non-agent) section: `[ask]`. Carries a
/// validated provider choice — nothing to interpolate and no credentials
/// surface in v1 (the section has no settings beyond the provider name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskSection {
    pub provider: AskProviderKind,
}

/// A resolved registry: user entries merged with project-wins precedence
/// (per agent name, and for the `[ask]` section wholesale).
#[derive(Debug, Default, Clone)]
pub struct Registry {
    agents: BTreeMap<String, AgentSpec>,
    ask: Option<AskSection>,
}

/// One parsed registry layer: its agent map plus the global `[ask]`
/// section (either may be empty/absent). `Registry::from_layers` merges
/// layers; parsing TOML into them is the config adapter's job.
#[derive(Debug, Default, Clone)]
pub struct RegistryLayer {
    pub agents: BTreeMap<String, AgentSpec>,
    pub ask: Option<AskSection>,
}

impl Registry {
    /// Merge already-parsed registry layers: project entries replace user
    /// entries per agent name (other entries merge), and the project
    /// `[ask]` section replaces the user's wholesale (the same rule the
    /// per-agent-name merge follows — a section is never merged key by
    /// key). (Parsing TOML into layers is `config_fs`'s job.)
    pub fn from_layers(
        user: Option<RegistryLayer>,
        project: Option<RegistryLayer>,
    ) -> Self {
        // Wholesale section replacement: project.or(user) — extracted
        // before the layers are consumed by the agent merge below.
        let ask = project
            .as_ref()
            .and_then(|l| l.ask.clone())
            .or_else(|| user.as_ref().and_then(|l| l.ask.clone()));
        let mut agents = BTreeMap::new();
        for layer in [user, project].into_iter().flatten() {
            for (name, spec) in layer.agents {
                agents.insert(name, spec); // project processed last: wins wholesale
            }
        }
        Self { agents, ask }
    }

    /// Resolve an agent by name, interpolating `${VAR}` from the process
    /// environment. Errors name the unresolved agent.
    pub fn resolve(&self, name: &str) -> Result<AgentSpec, ConfigError> {
        self.resolve_with(name, &|var| std::env::var(var).ok())
    }

    /// Like [`Registry::resolve`] with an explicit environment lookup
    /// (used by tests and callers that need deterministic interpolation).
    pub fn resolve_with(
        &self,
        name: &str,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Result<AgentSpec, ConfigError> {
        match self.agents.get(name) {
            Some(spec) => Ok(spec.interpolate(lookup)),
            None => Err(ConfigError::UnknownAgent {
                name: name.to_string(),
            }),
        }
    }

    /// The authored (pre-interpolation) spec for an agent, when it is
    /// registered. Unlike [`Registry::resolve`]/[`Registry::resolve_with`],
    /// which substitute `${VAR}` eagerly, this returns the spec exactly
    /// as written — the view a caller needs when a resolved value would
    /// carry a secret (the run record's `command`/`args`).
    pub fn raw(&self, name: &str) -> Option<&AgentSpec> {
        self.agents.get(name)
    }

    /// Names of all registered agents.
    pub fn agent_names(&self) -> Vec<String> {
        self.agents.keys().cloned().collect()
    }

    /// The effective `[ask]` section (merged across layers wholesale),
    /// when any layer defined one.
    pub fn ask(&self) -> Option<&AskSection> {
        self.ask.as_ref()
    }
}

#[derive(Debug)]
pub enum ConfigError {
    UnknownAgent { name: String },
    Parse { label: String, source: String },
    Io { label: String, source: String },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::UnknownAgent { name } => write!(
                f,
                "unknown agent `{name}`: not found in the user or project registry"
            ),
            ConfigError::Parse { label, source } => {
                write!(f, "invalid {label} config: {source}")
            }
            ConfigError::Io { label, source } => {
                write!(f, "failed to read {label} config: {source}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(command: &str, args: &[&str], env: &[(&str, &str)]) -> AgentSpec {
        AgentSpec {
            command: command.into(),
            args: args.iter().map(std::string::ToString::to_string).collect(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn layers() -> (RegistryLayer, RegistryLayer) {
        let user = RegistryLayer {
            agents: BTreeMap::from([
                (
                    "claude".to_string(),
                    spec("claude-acp-user", &["--old"], &[("MODEL", "sonnet")]),
                ),
                ("shared".to_string(), spec("shared-bin", &[], &[])),
            ]),
            ask: Some(AskSection {
                provider: AskProviderKind::None,
            }),
        };
        let project = RegistryLayer {
            agents: BTreeMap::from([
                (
                    "claude".to_string(),
                    spec("claude-acp-project", &["--new"], &[]),
                ),
                ("gemini".to_string(), spec("gemini-acp", &[], &[])),
            ]),
            ask: None,
        };
        (user, project)
    }

    #[test]
    fn project_overrides_user_wholesale() {
        let (user, project) = layers();
        let reg = Registry::from_layers(Some(user), Some(project));
        let resolved = reg.resolve_with("claude", &|_| None).unwrap();
        // Project definition wins; user fields (env MODEL) are NOT inherited.
        assert_eq!(resolved.command, "claude-acp-project");
        assert_eq!(resolved.args, vec!["--new"]);
        assert!(
            resolved.env.is_empty(),
            "user env must not leak: {resolved:?}"
        );
    }

    #[test]
    fn disjoint_agents_merge() {
        let (user, project) = layers();
        let reg = Registry::from_layers(Some(user), Some(project));
        assert_eq!(
            reg.resolve_with("gemini", &|_| None).unwrap().command,
            "gemini-acp"
        );
        assert_eq!(
            reg.resolve_with("shared", &|_| None).unwrap().command,
            "shared-bin"
        );
    }

    #[test]
    fn no_registry_errors_naming_agent() {
        let reg = Registry::from_layers(None, None);
        let err = reg.resolve_with("claude", &|_| None).unwrap_err();
        assert!(
            err.to_string().contains("claude"),
            "error must name the agent: {err}"
        );
    }

    #[test]
    fn raw_returns_authored_text_while_resolve_interpolates() {
        // The secrets boundary: `raw` is the pre-interpolation view the
        // run record reads, while `resolve` substitutes `${VAR}`.
        let reg = Registry::from_layers(
            None,
            Some(RegistryLayer {
                agents: BTreeMap::from([(
                    "api".to_string(),
                    spec(
                        "${BIN}",
                        &["--key", "${ANTHROPIC_API_KEY}"],
                        &[("TOKEN", "${ANTHROPIC_API_KEY}")],
                    ),
                )]),
                ask: None,
            }),
        );

        let authored = reg.raw("api").expect("registered agent");
        assert_eq!(authored.command, "${BIN}");
        assert_eq!(authored.args, vec!["--key", "${ANTHROPIC_API_KEY}"]);
        assert_eq!(authored.env["TOKEN"], "${ANTHROPIC_API_KEY}");

        let resolved = reg
            .resolve_with("api", &|var| match var {
                "BIN" => Some("/usr/local/bin/agent".into()),
                "ANTHROPIC_API_KEY" => Some("sk-secret".into()),
                _ => None,
            })
            .unwrap();
        assert_eq!(resolved.command, "/usr/local/bin/agent");
        assert_eq!(resolved.args, vec!["--key", "sk-secret"]);
        assert_eq!(resolved.env["TOKEN"], "sk-secret");

        assert!(reg.raw("missing").is_none());
    }

    #[test]
    fn interpolate_set_unset_and_embedded() {
        let s = spec(
            "${HOME}/bin/agent",
            &["--key=${MISSING_KEY}", "a${X}b"],
            &[("M", "${MODEL}")],
        );
        let lookup = |v: &str| -> Option<String> {
            match v {
                "HOME" => Some("/home/pat".into()),
                "X" => Some("mid".into()),
                "MODEL" => Some("opus".into()),
                _ => None,
            }
        };
        let out = s.interpolate(&lookup);
        assert_eq!(out.command, "/home/pat/bin/agent");
        assert_eq!(out.args[0], "--key=");
        assert_eq!(out.args[1], "amidb");
        assert_eq!(out.env["M"], "opus");
    }

    // ------------------------------------------------------------------
    // [ask] section merging (ask capability: wholesale layer rule)
    // ------------------------------------------------------------------

    fn ask_layer(
        agents: &[(&str, &str)],
        ask: Option<AskProviderKind>,
    ) -> RegistryLayer {
        RegistryLayer {
            agents: agents
                .iter()
                .map(|(name, cmd)| (name.to_string(), spec(cmd, &[], &[])))
                .collect(),
            ask: ask.map(|provider| AskSection { provider }),
        }
    }

    #[test]
    fn project_ask_section_replaces_user_ask_section_wholesale() {
        let reg = Registry::from_layers(
            Some(ask_layer(&[], Some(AskProviderKind::None))),
            Some(ask_layer(&[], Some(AskProviderKind::Stdin))),
        );
        assert_eq!(
            reg.ask().map(|a| a.provider),
            Some(AskProviderKind::Stdin),
            "project section must win in its entirety"
        );
    }

    #[test]
    fn user_only_ask_section_applies() {
        let reg = Registry::from_layers(
            Some(ask_layer(&[], Some(AskProviderKind::None))),
            Some(ask_layer(&[], None)),
        );
        assert_eq!(
            reg.ask().map(|a| a.provider),
            Some(AskProviderKind::None)
        );
    }

    #[test]
    fn no_ask_section_anywhere_is_none() {
        let reg = Registry::from_layers(
            Some(ask_layer(&[], None)),
            Some(ask_layer(&[], None)),
        );
        assert!(reg.ask().is_none());
        assert!(Registry::from_layers(None, None).ask().is_none());
    }

    #[test]
    fn unknown_ask_provider_value_is_rejected() {
        let err = "stdni"
            .parse::<AskProviderKind>()
            .expect_err("unknown value must fail");
        assert!(err.contains("stdni"), "error must name the value: {err}");
        assert!(
            err.contains("stdin") && err.contains("none"),
            "error must name accepted values: {err}"
        );
        for known in ["stdin", "none"] {
            assert!(known.parse::<AskProviderKind>().is_ok(), "{known}");
        }
    }
}
