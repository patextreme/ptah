//! The manifest: skeleton generation for `ptah init`, default-index
//! injection for resolution, and toml_edit-based surgical edits that
//! preserve everything ptah does not own.
//!
//! Pesde's `Manifest` does not round-trip the `[indices]` table (the
//! field is `skip_serializing`), so — like pesde's own CLI — every
//! manifest write goes through [`toml_edit`] on the raw text, never
//! through `Manifest` serialization.

use pesde::manifest::target::TargetKind;
use pesde::manifest::Manifest;
use toml_edit::{DocumentMut, Item};

use crate::{Error, PackageProject};

/// Sanitize a directory name into a valid Pesde package-name segment
/// (`[a-z0-9_]`, 1–32 chars, no leading/trailing underscore, not all
/// digits). Falls back to `project` when nothing valid survives
/// (empty or digits-only names).
pub fn sanitize_name_segment(dir_name: &str) -> String {
    let mut name: String = dir_name
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let all_digits = !name.is_empty() && name.chars().all(|c| c.is_ascii_digit());
    name.truncate(32);
    while name.starts_with('_') {
        name.remove(0);
    }
    while name.ends_with('_') {
        name.pop();
    }
    if name.is_empty() || all_digits {
        "project".to_string()
    } else {
        name
    }
}

/// The `.ptah/pesde.toml` skeleton `ptah init` writes: a private
/// package named `components/<sanitized directory name>` targeting the
/// `luau` environment, with no dependencies and no `[indices]` table
/// (ptah supplies the default index at resolve time; the comments
/// document how to override it).
pub fn skeleton(dir_name: &str) -> String {
    format!(
        r#"# ptah package manifest (Pesde format; managed by `ptah package`).
#
# Dependencies live under [dependencies], keyed by the alias scripts
# require them as (`require("@alias")`). ptah edits only the
# dependency tables — everything else in this file is yours.
#
# Usually you add packages with commands, not by hand:
#
#   ptah package add <scope>/<name>          from the default registry
#   ptah package add --git <url> --rev <rev> from a git repository
#   ptah package add --path ../local-pkg     from a local directory
#
# The default registry index is supplied by ptah at resolve time; add
# an [indices] table only to override it:
#
#   [indices]
#   default = "https://github.com/pesde-pkg/index"

name = "components/{name}"
version = "0.1.0"
private = true

[target]
environment = "luau"
"#,
        name = sanitize_name_segment(dir_name)
    )
}

/// Read and parse the manifest through the pesde engine, mapping a
/// missing file and parse failures to their adapter errors.
pub async fn deser_manifest(project: &PackageProject) -> Result<Manifest, Error> {
    match project.pesde().deser_manifest().await {
        Ok(manifest) => Ok(manifest),
        Err(e) => match &e {
            pesde::errors::ManifestReadError::Io(io)
                if io.kind() == std::io::ErrorKind::NotFound =>
            {
                Err(Error::ManifestMissing {
                    ptah_dir: project.ptah_dir(),
                })
            }
            pesde::errors::ManifestReadError::Serde(path, source) => Err(Error::ManifestParse {
                source: format!("{source} (in {})", path.display()),
            }),
            pesde::errors::ManifestReadError::Io(io) => Err(Error::Io {
                context: "cannot read .ptah/pesde.toml".to_string(),
                source: std::io::Error::other(io.to_string()),
            }),
            _ => Err(Error::ManifestParse {
                source: e.to_string(),
            }),
        },
    }
}

/// Reject non-`luau` targets: ptah manages Luau packages only, and
/// other environments drag Roblox/Wally build machinery ptah does not
/// fund.
pub fn require_luau_target(manifest: &Manifest) -> Result<(), Error> {
    match manifest.target.kind() {
        TargetKind::Luau => Ok(()),
        other => Err(Error::UnsupportedTarget {
            found: other.to_string(),
        }),
    }
}

// ----------------------------------------------------------------------
// Textual manifest I/O (toml_edit)
// ----------------------------------------------------------------------

/// The dependency table ptah's package commands write into. Peer and
/// dev tables exist in the Pesde manifest format, but ptah only ever
/// records standard dependencies (and preserves the others untouched).
const DEPENDENCIES_KEY: &str = "dependencies";

/// Read the raw manifest text.
pub fn read_manifest_text(project: &PackageProject) -> Result<String, Error> {
    let path = project.ptah_dir().join("pesde.toml");
    std::fs::read_to_string(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Error::ManifestMissing {
            ptah_dir: project.ptah_dir(),
        },
        _ => Error::Io {
            context: format!("cannot read {}", path.display()),
            source: e,
        },
    })
}

/// Write the raw manifest text.
pub fn write_manifest_text(project: &PackageProject, text: &str) -> Result<(), Error> {
    let path = project.ptah_dir().join("pesde.toml");
    std::fs::write(&path, text).map_err(|e| Error::Io {
        context: format!("cannot write {}", path.display()),
        source: e,
    })
}

/// Parse manifest text into a document-preserving toml_edit document.
/// The adapter never deserializes-and-reserializes the whole manifest
/// (Pesde's `Manifest` is lossy by design — `indices` is
/// `skip_serializing`), so every edit is surgical over the text.
pub fn parse_manifest_doc(text: &str) -> Result<DocumentMut, Error> {
    text.parse::<DocumentMut>().map_err(|e| Error::ManifestParse {
        source: e.to_string(),
    })
}

/// A dependency entry as ptah records it in the manifest: exactly the
/// fields each source form needs, matching Pesde's own `add` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyEntry {
    /// A registry dependency: `name`, caret `version`, and (only when
    /// they differ from the defaults) `target` and `index`.
    Pesde {
        name: String,
        version: String,
        target: Option<String>,
        index: Option<String>,
    },
    /// A git dependency: `repo`, resolved `rev`, optional `path`
    /// subdirectory.
    Git {
        repo: String,
        rev: String,
        path: Option<String>,
    },
    /// A local path dependency (absolute, so resolution is
    /// invocation-directory independent).
    Path { path: String },
}

impl DependencyEntry {
    /// A one-line human summary for command output.
    pub fn summary(&self) -> String {
        match self {
            DependencyEntry::Pesde { name, version, .. } => format!("{name} {version}"),
            DependencyEntry::Git { repo, rev, path } => match path {
                Some(path) => format!("{repo}#{rev} ({path})"),
                None => format!("{repo}#{rev}"),
            },
            DependencyEntry::Path { path } => format!("path {path}"),
        }
    }
}

/// Record `alias -> entry` under `[dependencies]`, preserving every
/// other byte of the document (user tables, comments, formatting).
pub fn set_dependency(doc: &mut DocumentMut, alias: &str, entry: &DependencyEntry) {
    let table = doc[DEPENDENCIES_KEY]
        .or_insert(Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .expect("dependencies is a table (just inserted)");
    // A per-alias table keeps a uniform shape even for a single field
    // (path deps) and matches Pesde's manifest conventions.
    let field = &mut table[alias];
    let field_table = field.or_insert(Item::Table(toml_edit::Table::new()));
    let field_table = field_table
        .as_table_mut()
        .expect("dependency entry is a table (just inserted)");
    match entry {
        DependencyEntry::Pesde {
            name,
            version,
            target,
            index,
        } => {
            field_table["name"] = toml_edit::value(name);
            field_table["version"] = toml_edit::value(version);
            if let Some(target) = target {
                field_table["target"] = toml_edit::value(target);
            }
            if let Some(index) = index {
                field_table["index"] = toml_edit::value(index);
            }
        }
        DependencyEntry::Git { repo, rev, path } => {
            field_table["repo"] = toml_edit::value(repo);
            field_table["rev"] = toml_edit::value(rev);
            if let Some(path) = path {
                field_table["path"] = toml_edit::value(path);
            }
        }
        DependencyEntry::Path { path } => {
            field_table["path"] = toml_edit::value(path);
        }
    }
}

/// Remove the dependency with `alias` from whichever dependency table
/// carries it. Returns `false` when no table had it.
pub fn remove_dependency(doc: &mut DocumentMut, alias: &str) -> bool {
    ["dependencies", "peer_dependencies", "dev_dependencies"].iter().any(|key| {
        doc[key]
            .as_table_mut()
            .is_some_and(|table| table.remove(alias).is_some())
    })
}

/// The alias set from the manifest's dependency tables (the source
/// for `.luaurc` sync): every key of `dependencies`,
/// `peer_dependencies`, and `dev_dependencies`, mapped to its
/// `luau_packages` linker-module path.
pub fn dependency_aliases(text: &str) -> Result<std::collections::BTreeMap<String, String>, Error> {
    let doc = parse_manifest_doc(text)?;
    let mut aliases = std::collections::BTreeMap::new();
    for table in ["dependencies", "peer_dependencies", "dev_dependencies"] {
        if let Some(dep_table) = doc.get(table).and_then(Item::as_table) {
            for (alias, _entry) in dep_table.iter() {
                aliases.insert(
                    alias.to_string(),
                    crate::driver::linker_module_path(alias),
                );
            }
        }
    }
    Ok(aliases)
}

/// Whether the document's `[indices]` table carries any entry (i.e.
/// the user configured registry indices of their own).
pub fn has_user_indices(doc: &DocumentMut) -> bool {
    doc.get("indices")
        .and_then(Item::as_table)
        .is_some_and(|table| table.iter().any(|(_, item)| item.as_value().is_some()))
}

/// Materialize ptah's default index into the document when (and only
/// when) the manifest declares no `[indices]` entries: resolution
/// against Pesde's registry needs the index reachable from the
/// manifest text. Returns `true` when something was injected — the
/// caller restores the original text after the operation, so the
/// user's file never keeps a ptah-written indices table. A user table
/// with any entry wins wholesale and is left untouched.
pub fn materialize_default_index(doc: &mut DocumentMut) -> bool {
    if has_user_indices(doc) {
        return false;
    }
    doc["indices"]["default"] = toml_edit::value(crate::default_index_url());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse `text` through pesde's own `Manifest` deserialization by
    /// round-tripping a real project directory.
    async fn parse(text: &str) -> Manifest {
        let dir = tempfile::tempdir().unwrap();
        let ptah = dir.path().join(".ptah");
        std::fs::create_dir_all(&ptah).unwrap();
        std::fs::write(ptah.join("pesde.toml"), text).unwrap();
        let project = PackageProject::open(dir.path());
        deser_manifest(&project).await.expect("manifest must parse")
    }

    #[tokio::test]
    async fn skeleton_parses_as_a_valid_private_luau_manifest() {
        let manifest = parse(&skeleton("My_Project!")).await;
        assert!(manifest.private, "skeleton is private");
        assert_eq!(manifest.target.kind(), TargetKind::Luau);
        assert!(
            manifest.dependencies.is_empty()
                && manifest.peer_dependencies.is_empty()
                && manifest.dev_dependencies.is_empty(),
            "skeleton has no dependency entries"
        );
        assert!(manifest.indices.is_empty(), "skeleton has no indices");
        assert_eq!(manifest.name.to_string(), "components/my_project");
    }

    #[tokio::test]
    async fn skeleton_carries_no_indices_table_textually() {
        // No uncommented `[indices]` table may appear in the file, so
        // user-added entries can never collide with ptah-written ones
        // (the docs comment may *mention* it).
        let text = skeleton("anything");
        let active = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with('#'))
            .collect::<Vec<_>>();
        assert!(
            !active.iter().any(|l| l.contains("[indices]")),
            "indices table leaked into the skeleton: {text}"
        );
        assert!(text.contains("environment = \"luau\""), "{text}");
        assert!(text.contains("private = true"), "{text}");
    }

    #[test]
    fn name_sanitization_covers_the_pesde_rules() {
        // Plain lowercase passes through.
        assert_eq!(sanitize_name_segment("my-project"), "my_project");
        // Case-folding, invalid chars mapped, no lead/trail underscore.
        assert_eq!(sanitize_name_segment("My Project!"), "my_project");
        assert_eq!(sanitize_name_segment("--x--"), "x");
        // Long names truncate to the 32-char limit without a trailing
        // underscore.
        let long = "a".repeat(40);
        assert_eq!(sanitize_name_segment(&long).len(), 32);
        // Empty / underscore-only / digits-only fall back.
        for dir_name in ["", "___", "123", "42"] {
            assert_eq!(sanitize_name_segment(dir_name), "project");
        }
        // The sanitized result always parses as a name segment.
        for dir_name in ["a", "A-b.c", "2fast2furious"] {
            let seg = sanitize_name_segment(dir_name);
            let full = format!("components/{seg}");
            assert_eq!(
                full.parse::<pesde::names::PackageName>().unwrap().to_string(),
                full,
                "{dir_name} -> {seg}"
            );
        }
    }

    #[tokio::test]
    async fn unparseable_manifest_is_a_usage_error() {
        let dir = tempfile::tempdir().unwrap();
        let ptah = dir.path().join(".ptah");
        std::fs::create_dir_all(&ptah).unwrap();
        std::fs::write(ptah.join("pesde.toml"), "not toml {{{").unwrap();
        let project = PackageProject::open(dir.path());
        let err = deser_manifest(&project).await.unwrap_err();
        assert!(matches!(err, Error::ManifestParse { .. }), "{err}");
        assert!(err.is_usage());
    }

    #[tokio::test]
    async fn missing_manifest_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".ptah")).unwrap();
        let project = PackageProject::open(dir.path());
        let err = deser_manifest(&project).await.unwrap_err();
        assert!(matches!(err, Error::ManifestMissing { .. }), "{err}");
        assert!(err.to_string().contains("pesde.toml"), "{err}");
    }

    #[test]
    fn target_gate_rejects_non_luau_environments() {
        let manifest = toml::from_str::<Manifest>(
            r#"
name = "abc/roblox_thing"
version = "0.1.0"

[target]
environment = "roblox"
"#,
        )
        .unwrap();
        let err = require_luau_target(&manifest).unwrap_err();
        assert!(matches!(err, Error::UnsupportedTarget { .. }), "{err}");
        assert!(!err.is_usage());
        assert!(err.to_string().contains("roblox"), "{err}");
    }

    // ------------------------------------------------------------------
    // Textual manifest edits (toml_edit)
    // ------------------------------------------------------------------

    const USER_MANIFEST: &str = r#"# my project manifest
name = "abc/my_project"
version = "0.2.0"
private = true

[target]
environment = "luau"

# my own index
[indices]
default = "https://example.com/custom-index"
other = "https://example.com/other"
"#;

    #[test]
    fn user_indices_table_round_trips_byte_identically_through_edits() {
        let mut doc = parse_manifest_doc(USER_MANIFEST).unwrap();
        set_dependency(
            &mut doc,
            "hello",
            &DependencyEntry::Pesde {
                name: "pesde/hello".into(),
                version: "^0.2.0".into(),
                target: None,
                index: None,
            },
        );
        let written = doc.to_string();
        // The user's [indices] table (comments included) survives the
        // dependency edit byte-for-byte: the section from its comment
        // through the last entry is identical; only the new
        // [dependencies] table is appended after it.
        let indices_start = USER_MANIFEST.find("# my own index").unwrap();
        let original_section = &USER_MANIFEST[indices_start..];
        let written_section = &written[written.find("# my own index").unwrap()..];
        assert!(
            written_section.starts_with(original_section),
            "indices table must round-trip byte-identically (only new\n             dependency tables may follow): {written}"
        );
        // The dependency landed.
        assert!(written.contains("[dependencies.hello]"), "{written}");
        assert!(written.contains("version = \"^0.2.0\""), "{written}");
        // And the edited manifest still parses as a Pesde manifest
        // with the dependency present.
        let parsed = toml::from_str::<Manifest>(&written).unwrap();
        assert_eq!(parsed.indices.len(), 2, "user indices preserved");
        assert_eq!(
            parsed.indices["default"].to_string(),
            "https://example.com/custom-index"
        );
        assert!(parsed.dependencies.contains_key(
            &"hello".parse::<pesde::manifest::Alias>().unwrap()
        ));
    }

    #[test]
    fn unknown_tables_and_comments_survive_dependency_edits() {
        let manifest = r#"name = "abc/x"
version = "0.1.0"

# a user field
[extra_table]
keep = "me"

[target]
environment = "luau"
"#;
        let mut doc = parse_manifest_doc(manifest).unwrap();
        set_dependency(
            &mut doc,
            "local",
            &DependencyEntry::Path {
                path: "/abs/pkg".into(),
            },
        );
        let written = doc.to_string();
        assert!(written.contains("# a user field"), "{written}");
        assert!(written.contains("keep = \"me\""), "{written}");
        assert!(written.contains("path = \"/abs/pkg\""), "{written}");
        toml::from_str::<Manifest>(&written).unwrap();
    }

    #[test]
    fn git_entries_record_repo_rev_and_path() {
        let mut doc = parse_manifest_doc(&skeleton("x")).unwrap();
        set_dependency(
            &mut doc,
            "hello",
            &DependencyEntry::Git {
                repo: "https://example.com/repo".into(),
                rev: "deadbeef".into(),
                path: Some("pkg/hello".into()),
            },
        );
        let written = doc.to_string();
        assert!(written.contains("repo = \"https://example.com/repo\""), "{written}");
        assert!(written.contains("rev = \"deadbeef\""), "{written}");
        assert!(written.contains("path = \"pkg/hello\""), "{written}");
        toml::from_str::<Manifest>(&written).unwrap();
    }

    #[test]
    fn remove_dependency_finds_and_deletes_across_tables() {
        let mut doc = parse_manifest_doc(
            r#"
name = "abc/x"
version = "0.1.0"

[target]
environment = "luau"

[dependencies]
hello = { name = "pesde/hello", version = "^0.1.0" }

[dev_dependencies]
tooling = { name = "pesde/tooling", version = "^1.0.0" }
"#,
        )
        .unwrap();
        assert!(remove_dependency(&mut doc, "hello"));
        assert!(!remove_dependency(&mut doc, "hello"));
        assert!(remove_dependency(&mut doc, "tooling"));
        assert!(!remove_dependency(&mut doc, "never_there"));
        let written = doc.to_string();
        assert!(!written.contains("hello"), "{written}");
        toml::from_str::<Manifest>(&written).unwrap();
    }

    #[test]
    fn default_index_injection_only_when_no_user_indices() {
        // No [indices] at all → injected, and the result parses with
        // the default index present.
        let mut doc = parse_manifest_doc(&skeleton("x")).unwrap();
        assert!(materialize_default_index(&mut doc));
        let injected = doc.to_string();
        let parsed = toml::from_str::<Manifest>(&injected).unwrap();
        assert_eq!(
            parsed.indices["default"].to_string(),
            crate::DEFAULT_INDEX_URL
        );

        // User [indices] → untouched (no injection, byte-identical).
        let mut doc = parse_manifest_doc(USER_MANIFEST).unwrap();
        assert!(!materialize_default_index(&mut doc));
        assert_eq!(doc.to_string(), USER_MANIFEST);

        // An empty [indices] table has no entries → injection adds the
        // default into it.
        let mut doc = parse_manifest_doc(
            "name = \"abc/x\"\nversion = \"0.1.0\"\n\n[target]\nenvironment = \"luau\"\n\n[indices]\n",
        )
        .unwrap();
        assert!(materialize_default_index(&mut doc));
        let parsed = toml::from_str::<Manifest>(&doc.to_string()).unwrap();
        assert_eq!(parsed.indices.len(), 1);
    }
}
