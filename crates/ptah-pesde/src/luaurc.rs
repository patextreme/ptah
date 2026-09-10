//! `.luaurc` synchronization: the project-root alias configuration is a
//! derived artifact of the package manifest, the way `.ptah/ptah.d.luau`
//! is a derived artifact of the binary.
//!
//! Package commands that change the installed dependency set recompute
//! the alias map (`<alias> -> ./.ptah/luau_packages/<alias>`) and sync
//! it into `<project>/.luaurc`: created when absent, otherwise
//! parse-modify-serialized — only entries whose values point into the
//! project's `luau_packages/` are ptah-owned and replaced or removed;
//! every other key (inside and outside `aliases`) survives untouched.
//! A user alias colliding with a package alias is reported and the
//! user's entry wins (the package's `@alias` require will fail loudly
//! at resolution instead of silently shadowing user configuration).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};

use crate::{Error, PackageProject};

/// What one sync did, for the CLI's diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LuaurcSync {
    /// The file was created (it did not exist).
    pub created: bool,
    /// The file was rewritten (entries added, updated, or removed).
    pub updated: bool,
    /// Aliases where a user-owned entry shadowed the package alias —
    /// reported as diagnostics; the user's entry stays untouched.
    pub collisions: Vec<String>,
}

/// Synchronize the package aliases into `<project>/.luaurc`.
pub fn sync(project: &PackageProject, aliases: &BTreeMap<String, String>) -> Result<LuaurcSync, Error> {
    let path = project.root().join(".luaurc");

    // Fresh file: write the alias map directly.
    if !path.exists() {
        let mut root = Map::new();
        root.insert("aliases".to_string(), Value::Object(alias_map(aliases)));
        write_luaurc(&path, &Value::Object(root))?;
        return Ok(LuaurcSync {
            created: true,
            updated: true,
            collisions: Vec::new(),
        });
    }

    let text = std::fs::read_to_string(&path).map_err(|e| Error::LuaurcSync {
        source: format!("cannot read {}: {e}", path.display()),
    })?;
    let mut doc: Value = serde_json::from_str(&text).map_err(|e| Error::LuaurcSync {
        source: format!(
            "{} is not valid JSON (.luaurc is a JSON file; fix or remove it): {e}",
            path.display()
        ),
    })?;

    let Some(root) = doc.as_object_mut() else {
        return Err(Error::LuaurcSync {
            source: format!(
                "{} must be a JSON object at the top level",
                path.display()
            ),
        });
    };
    let entries = root
        .entry("aliases".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entries.is_object() {
        return Err(Error::LuaurcSync {
            source: format!(
                "{}: `aliases` must be a JSON object of name -> path",
                path.display()
            ),
        });
    }
    let entries = entries.as_object_mut().expect("just checked");

    // One ptah-owned value per alias: the fixed linker-module path.
    let desired = alias_map(aliases);
    let mut report = LuaurcSync::default();

    // Remove ptah-owned entries for aliases no longer in the manifest.
    let stale: Vec<String> = entries
        .iter()
        .filter(|(name, value)| {
            is_ptah_owned(value) && !desired.contains_key(name.as_str())
        })
        .map(|(name, _)| name.clone())
        .collect();
    for name in &stale {
        entries.remove(name);
        report.updated = true;
    }

    // Add or update entries for the manifest's aliases; user-owned
    // entries win over package aliases (collision -> diagnostic).
    for (name, target) in &desired {
        let target = target.as_str().expect("alias map values are strings");
        match entries.get(name) {
            None => {
                entries.insert(name.clone(), Value::String(target.to_string()));
                report.updated = true;
            }
            Some(existing) if is_ptah_owned(existing) => {
                if existing.as_str() != Some(target) {
                    entries.insert(name.clone(), Value::String(target.to_string()));
                    report.updated = true;
                }
            }
            Some(_user_owned) => {
                report.collisions.push(name.clone());
            }
        }
    }

    if report.updated {
        write_luaurc(&path, &doc)?;
    }
    Ok(report)
}

/// The ptah-written alias map: alias -> `./.ptah/luau_packages/<alias>`.
fn alias_map(aliases: &BTreeMap<String, String>) -> Map<String, Value> {
    aliases
        .iter()
        .map(|(name, target)| (name.clone(), Value::String(target.clone())))
        .collect()
}

/// Whether an existing alias entry value is ptah-owned: a relative
/// path pointing into this project's `luau_packages/` directory
/// (`./`-form as ptah writes it; bare or `../`-anchored paths that
/// normalize inside it count too).
fn is_ptah_owned(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|v| looks_into_luau_packages(v))
}

/// Does `value` (an alias target string) resolve inside
/// `<root>/.ptah/luau_packages/`? Pure lexical check, mirroring how
/// the runtime anchors `./` targets at the config directory.
fn looks_into_luau_packages(value: &str) -> bool {
    if value.starts_with('/') || value.contains("://") {
        return false;
    }
    let joined = Path::new(".").join(value.trim_start_matches("./"));
    let mut normalized = std::path::PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let mut segments = Vec::new();
    for component in normalized.components() {
        if let std::path::Component::Normal(seg) = component {
            segments.push(seg.to_string_lossy().to_string());
        }
    }
    segments.first().is_some_and(|s| s == ".ptah")
        && segments.get(1).is_some_and(|s| s == "luau_packages")
        && segments.len() > 2
}

/// Serialize and write, pretty-printed with a trailing newline (the
/// form ptah always writes; user formatting of untouched entries is
/// preserved content-wise, not byte-wise).
fn write_luaurc(path: &Path, doc: &Value) -> Result<(), Error> {
    let text = serde_json::to_string_pretty(doc).map_err(|e| Error::LuaurcSync {
        source: format!("cannot serialize {}: {e}", path.display()),
    })?;
    std::fs::write(path, format!("{text}\n")).map_err(|e| Error::LuaurcSync {
        source: format!("cannot write {}: {e}", path.display()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (tempfile::TempDir, PackageProject) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".ptah")).unwrap();
        let project = PackageProject::open(dir.path());
        (dir, project)
    }

    fn read(dir: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join(".luaurc")).unwrap()).unwrap()
    }

    fn pkg_aliases(names: &[&str]) -> BTreeMap<String, String> {
        names
            .iter()
            .map(|n| ((*n).to_string(), crate::driver::linker_module_path(n)))
            .collect()
    }

    #[test]
    fn absent_file_is_created_with_the_alias_map() {
        let (dir, project) = project();
        let report = sync(&project, &pkg_aliases(&["hello"])).unwrap();
        assert!(report.created && report.updated);
        let doc = read(dir.path());
        assert_eq!(
            doc["aliases"]["hello"].as_str(),
            Some("./.ptah/luau_packages/hello")
        );
    }

    #[test]
    fn user_keys_and_user_aliases_are_preserved() {
        let (dir, project) = project();
        std::fs::write(
            dir.path().join(".luaurc"),
            r#"{
  "languageMode": "strict",
  "aliases": {
    "mine": "./my/own/module",
    "hello": "./.ptah/luau_packages/hello"
  }
}"#,
        )
        .unwrap();
        let report = sync(&project, &pkg_aliases(&["hello", "util"])).unwrap();
        assert!(!report.created, "{report:?}");
        assert!(report.updated, "adding `util` rewrites: {report:?}");
        let doc = read(dir.path());
        // The user key outside `aliases` and the user alias inside it
        // survive with their values; the new package alias was added.
        assert_eq!(doc["languageMode"].as_str(), Some("strict"));
        assert_eq!(doc["aliases"]["mine"].as_str(), Some("./my/own/module"));
        assert_eq!(
            doc["aliases"]["util"].as_str(),
            Some("./.ptah/luau_packages/util")
        );
        assert!(report.collisions.is_empty());
    }

    #[test]
    fn user_alias_collision_leaves_the_user_entry_untouched() {
        let (dir, project) = project();
        std::fs::write(
            dir.path().join(".luaurc"),
            r#"{"aliases": {"hello": "./custom/hello"}}"#,
        )
        .unwrap();
        let report = sync(&project, &pkg_aliases(&["hello"])).unwrap();
        assert_eq!(report.collisions, vec!["hello".to_string()]);
        let doc = read(dir.path());
        // User wins: the sync made no change, so the user's file is
        // untouched byte-for-byte.
        assert_eq!(doc["aliases"]["hello"].as_str(), Some("./custom/hello"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".luaurc")).unwrap(),
            "{\"aliases\": {\"hello\": \"./custom/hello\"}}"
        );
    }

    #[test]
    fn removing_the_dependency_deletes_the_ptah_owned_entry() {
        let (dir, project) = project();
        sync(&project, &pkg_aliases(&["hello", "gone"])).unwrap();
        // Second sync without `gone`.
        let report = sync(&project, &pkg_aliases(&["hello"])).unwrap();
        assert!(!report.created && report.updated);
        let doc = read(dir.path());
        assert!(doc["aliases"].get("gone").is_none());
        assert_eq!(
            doc["aliases"]["hello"].as_str(),
            Some("./.ptah/luau_packages/hello")
        );
        // And a no-op third sync changes nothing.
        let report = sync(&project, &pkg_aliases(&["hello"])).unwrap();
        assert!(!report.updated, "{report:?}");
    }

    #[test]
    fn ptah_owned_detection_is_lexical() {
        for (value, expect) in [
            ("./.ptah/luau_packages/hello", true),
            (".ptah/luau_packages/hello", true),
            ("./.ptah/luau_packages/nested/deep", true),
            ("./other/luau_packages/hello", false),
            ("./my/own/module", false),
            ("/abs/.ptah/luau_packages/hello", false),
            ("https://example.com", false),
        ] {
            let value = Value::String(value.to_string());
            assert_eq!(
                is_ptah_owned(&value),
                expect,
                "{value:?}"
            );
        }
    }

    #[test]
    fn unparseable_luaurc_is_an_error_naming_the_file() {
        let (dir, project) = project();
        std::fs::write(dir.path().join(".luaurc"), "not json").unwrap();
        let err = sync(&project, &pkg_aliases(&["hello"])).unwrap_err();
        assert!(matches!(err, Error::LuaurcSync { .. }), "{err}");
        assert!(err.to_string().contains(".luaurc"), "{err}");
        assert!(err.to_string().contains("JSON"), "{err}");
    }
}
