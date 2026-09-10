//! Registry file I/O: TOML parsing and layer discovery for the agent
//! registry model ([`ptah_core::config`]).
//!
//! Project entries (`.ptah/config.toml`, discovered upward from the
//! invocation directory) override user entries (`$XDG_CONFIG_HOME/ptah/`
//! or `~/.config/ptah/config.toml`) wholesale per agent name.

use std::path::{Path, PathBuf};

use ptah_core::config::{AgentSpec, AskProviderKind, AskSection, ConfigError, Registry, RegistryLayer};
use ptah_core::ports::ConfigSource;
use std::collections::BTreeMap;
use std::str::FromStr as _;

#[derive(Debug, Default, serde::Deserialize)]
struct RegistryFile {
    #[serde(default)]
    agents: BTreeMap<String, AgentSpec>,
    #[serde(default)]
    ask: Option<AskFile>,
}

/// The raw `[ask]` section: `provider` is validated against the known
/// value set at parse time (missing or unknown values fail discovery
/// with an error naming the file, the section, and the accepted
/// values), so a merged registry can never carry an unvalidated choice.
#[derive(Debug, Default, serde::Deserialize)]
struct AskFile {
    provider: Option<String>,
}

/// Parse one registry layer's TOML contents into its agents and its
/// `[ask]` section (when present, validated — see [`AskFile`]).
fn parse_layer(label: &str, contents: &str) -> Result<RegistryLayer, ConfigError> {
    let file: RegistryFile = toml::from_str(contents).map_err(|e| ConfigError::Parse {
        label: label.into(),
        source: e.to_string(),
    })?;
    let ask = match file.ask {
        None => None,
        Some(section) => {
            let provider = section.provider.as_deref().ok_or_else(|| ConfigError::Parse {
                label: label.into(),
                source: "[ask] section: missing required `provider` key \
                        (accepted values: `stdin`, `none`)"
                    .to_string(),
            })?;
            let provider = AskProviderKind::from_str(provider).map_err(|source| ConfigError::Parse {
                label: label.into(),
                source: format!("[ask] section: {source}"),
            })?;
            Some(AskSection { provider })
        }
    };
    Ok(RegistryLayer {
        agents: file.agents,
        ask,
    })
}

/// Parse a [`Registry`] from the contents of user and project config
/// files. Project entries replace user entries per agent name; other
/// entries merge.
///
/// A free function, not an inherent `Registry` method: TOML parsing is
/// this adapter's job and an inherent impl would be illegal across the
/// crate boundary (the model lives in `ptah-core`).
pub fn from_parts(user: Option<&str>, project: Option<&str>) -> Result<Registry, ConfigError> {
    let user = user.map(|c| parse_layer("user", c)).transpose()?;
    let project = project.map(|c| parse_layer("project", c)).transpose()?;
    Ok(Registry::from_layers(user, project))
}

/// Load from explicit file paths (missing files are fine).
pub fn load(user: Option<&Path>, project: Option<&Path>) -> Result<Registry, ConfigError> {
    let read = |p: Option<&Path>, label: &str| -> Result<Option<String>, ConfigError> {
        match p {
            None => Ok(None),
            Some(p) if !p.exists() => Ok(None),
            Some(p) => std::fs::read_to_string(p)
                .map(Some)
                .map_err(|source| ConfigError::Io {
                    label: label.into(),
                    source: source.to_string(),
                }),
        }
    };
    from_parts(
        read(user, "user")?.as_deref(),
        read(project, "project")?.as_deref(),
    )
}

/// Discover user (`$XDG_CONFIG_HOME/ptah` or `~/.config/ptah`) and
/// project (nearest ancestor `.ptah`, from the invocation directory)
/// registries.
pub fn discover(invocation_dir: &Path) -> Result<Registry, ConfigError> {
    let user = user_config_path();
    let project = find_project_config(invocation_dir);
    load(user.as_deref(), project.as_deref())
}

/// Filesystem-backed [`ConfigSource`]: TOML discovery and loading of the
/// user and project registry layers.
pub struct FsConfigSource;

impl ConfigSource for FsConfigSource {
    fn discover(&self, invocation_dir: &Path) -> Result<Registry, ConfigError> {
        discover(invocation_dir)
    }
}

/// `$XDG_CONFIG_HOME/ptah/config.toml` or `$HOME/.config/ptah/config.toml`.
pub fn user_config_path() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => {
            let home = std::env::var_os("HOME")?;
            PathBuf::from(home).join(".config")
        }
    };
    Some(base.join("ptah").join("config.toml"))
}

/// Nearest `.ptah/config.toml` in `start` or an ancestor directory.
pub fn find_project_config(start: &Path) -> Option<PathBuf> {
    let mut dir: &Path = start;
    loop {
        let candidate = dir.join(".ptah").join("config.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        {
            let parent = dir.parent()?;
            dir = parent;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = r#"
[agents.claude]
command = "claude-acp-user"
args = ["--old"]
env = { MODEL = "sonnet" }

[agents.shared]
command = "shared-bin"
"#;

    const PROJECT: &str = r#"
[agents.claude]
command = "claude-acp-project"
args = ["--new"]

[agents.gemini]
command = "gemini-acp"
"#;

    #[test]
    fn from_parts_merges_layers_with_project_winning() {
        let reg = from_parts(Some(USER), Some(PROJECT)).unwrap();
        let claude = reg.resolve_with("claude", &|_| None).unwrap();
        assert_eq!(claude.command, "claude-acp-project");
        assert!(claude.env.is_empty(), "user env must not leak: {claude:?}");
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
    fn missing_files_are_ok() {
        let reg = load(None, Some(Path::new("/nonexistent/x.toml"))).unwrap();
        assert!(reg.agent_names().is_empty());
        assert!(reg.ask().is_none());
    }

    #[test]
    fn parse_error_is_labeled() {
        let err = from_parts(Some("not toml {{{"), None).unwrap_err();
        assert!(err.to_string().contains("user"), "{err}");
    }

    // ------------------------------------------------------------------
    // [ask] section parsing (agent-registry capability)
    // ------------------------------------------------------------------

    #[test]
    fn valid_ask_section_parses() {
        for (contents, expect) in [
            (
                "[ask]\nprovider = \"stdin\"\n",
                ptah_core::config::AskProviderKind::Stdin,
            ),
            (
                "[ask]\nprovider = \"none\"\n",
                ptah_core::config::AskProviderKind::None,
            ),
        ] {
            let reg = from_parts(None, Some(contents)).unwrap();
            assert_eq!(
                reg.ask().map(|a| a.provider),
                Some(expect),
                "for {contents}"
            );
        }
    }

    #[test]
    fn unknown_ask_provider_fails_discovery_naming_layer_section_and_values() {
        let err = from_parts(Some("[ask]\nprovider = \"stdni\"\n"), None).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("user config"), "names the layer: {msg}");
        assert!(msg.contains("[ask]"), "names the section: {msg}");
        assert!(
            msg.contains("stdin") && msg.contains("none"),
            "names accepted values: {msg}"
        );
    }

    #[test]
    fn missing_provider_key_fails_discovery() {
        let err = from_parts(None, Some("[ask]\n")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("[ask]"), "{msg}");
        assert!(msg.contains("provider"), "{msg}");
    }

    #[test]
    fn absent_ask_section_leaves_agents_untouched() {
        // Agents-only files parse exactly as before; no ask data.
        let reg = from_parts(Some(USER), Some(PROJECT)).unwrap();
        assert!(reg.ask().is_none());
        assert_eq!(reg.agent_names().len(), 3);
    }

    #[test]
    fn project_ask_replaces_user_ask_wholesale() {
        let reg = from_parts(
            Some("[ask]\nprovider = \"none\"\n"),
            Some("[ask]\nprovider = \"stdin\"\n"),
        )
        .unwrap();
        assert_eq!(
            reg.ask().map(|a| a.provider),
            Some(ptah_core::config::AskProviderKind::Stdin)
        );
        // And user-only applies when the project defines none.
        let reg = from_parts(Some("[ask]\nprovider = \"none\"\n"), Some(PROJECT)).unwrap();
        assert_eq!(
            reg.ask().map(|a| a.provider),
            Some(ptah_core::config::AskProviderKind::None)
        );
    }
}
