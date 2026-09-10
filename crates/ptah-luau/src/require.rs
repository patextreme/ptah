//! Relative and alias module resolution for ptah scripts.
//!
//! Implements mlua's `Require` trait relative to the entry script's
//! directory. `require("./lib/util")` resolves `.luau`/`.lua`/`init.luau`
//! files relative to the requiring file, with no boundary: relative paths
//! may walk out of the entry script's directory to anywhere on disk.
//! `require("@hello")` resolves through `.luaurc` alias configuration:
//! mlua's navigator walks up from the requiring file's directory to the
//! nearest `.luaurc` (or `.config.luau`) carrying the alias, parses the
//! config itself, and hands the alias's target path back anchored at the
//! config's directory — standard Luau require-by-string semantics, the
//! same file editors and luau-lsp read. Require strings that are neither
//! relative nor alias form (absolute paths, bare module names) are
//! rejected with a Lua error. Caching (same path → same module table) is
//! provided by mlua's loader cache keyed on the resolved path.

use std::path::{Component, Path, PathBuf};
use std::result::Result as StdResult;

use mlua::luau::{NavigateError, Require};
use mlua::{Function, Lua};

/// Lexically normalize a path: resolve `.`/`..` components without
/// touching the filesystem (`..` at the root pops, matching the
/// navigator). Shared by the runtime navigator and the static checker.
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

/// The JSON configuration filename Luau's require-by-string system
/// discovers on the walk-up.
const LUAURC_CONFIG_FILENAME: &str = ".luaurc";
/// The Luau-source configuration filename discovered the same way.
const LUAU_CONFIG_FILENAME: &str = ".config.luau";

/// A requirer rooted at the entry script's directory.
#[derive(Debug, Clone)]
pub struct ScriptRequirer {
    /// Absolute directory of the entry script; relative chunk names are
    /// joined onto it.
    root: PathBuf,
    /// Absolute path (file or dir) the navigation currently points at.
    current: PathBuf,
}

impl ScriptRequirer {
    pub fn new(root: PathBuf) -> Self {
        Self {
            current: root.clone(),
            root,
        }
    }
}

impl Require for ScriptRequirer {
    fn is_require_allowed(&self, chunk_name: &str) -> bool {
        chunk_name.starts_with('@')
    }

    fn reset(&mut self, chunk_name: &str) -> StdResult<(), NavigateError> {
        let raw = chunk_name
            .strip_prefix('@')
            .ok_or(NavigateError::NotFound)?;
        // Chunk line suffixes ("file.luau:12") are not module paths.
        let raw = raw.rsplit_once(':').map_or(raw, |(p, _)| p);
        let path = normalize(Path::new(raw));
        let path = if path.is_absolute() {
            path
        } else {
            self.root.join(path)
        };
        self.current = path;
        Ok(())
    }

    fn jump_to_alias(&mut self, path: &str) -> StdResult<(), NavigateError> {
        // Alias targets that are neither `./`-relative nor further
        // aliases (absolute paths, bare names) do not anchor to the
        // configuration file's directory and are not supported — the
        // same rejection the runtime applies to non-relative require
        // strings.
        Err(NavigateError::Other(mlua::Error::runtime(format!(
            "alias target is not relative to the configuration file: `{path}` \
             (only \"./\" and \"../\" targets are allowed)"
        ))))
    }

    fn to_parent(&mut self) -> StdResult<(), NavigateError> {
        let mut path = self.current.clone();
        if !path.pop() {
            return Err(NavigateError::NotFound);
        }
        let path = normalize(&path);
        self.current = path;
        Ok(())
    }

    fn to_child(&mut self, name: &str) -> StdResult<(), NavigateError> {
        let path = normalize(&self.current.join(name));
        // A child that is neither a module nor a directory cannot be part of
        // any deeper resolution: fail with an error naming the path (per the
        // scripting spec) instead of a generic not-found.
        if resolve_file(&path).is_none() && !path.is_dir() {
            return Err(NavigateError::Other(mlua::Error::runtime(format!(
                "module not found: {}",
                path.display()
            ))));
        }
        self.current = path;
        Ok(())
    }

    fn has_module(&self) -> bool {
        resolve_file(&self.current).is_some()
    }

    fn cache_key(&self) -> String {
        resolve_file(&self.current)
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    }

    fn has_config(&self) -> bool {
        // Luau's navigator checks for configuration at the current
        // position while walking up from the requiring file — always a
        // directory at that point.
        self.current.is_dir()
            && (self.current.join(LUAURC_CONFIG_FILENAME).is_file()
                || self.current.join(LUAU_CONFIG_FILENAME).is_file())
    }

    fn config(&self) -> std::io::Result<Vec<u8>> {
        let path = self.current.join(LUAURC_CONFIG_FILENAME);
        if path.is_file() {
            return std::fs::read(path);
        }
        std::fs::read(self.current.join(LUAU_CONFIG_FILENAME))
    }

    fn loader(&self, lua: &Lua) -> mlua::Result<Function> {
        let path = resolve_file(&self.current).ok_or_else(|| {
            mlua::Error::runtime(format!("module not found: {}", self.current.display()))
        })?;
        let source = std::fs::read_to_string(&path).map_err(|e| {
            mlua::Error::runtime(format!("cannot read module {}: {e}", path.display()))
        })?;
        lua.load(source)
            .set_name(format!("@{}", path.display()))
            .into_function()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ptah-req-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::create_dir_all(dir.join("lib/sub")).unwrap();
        std::fs::write(dir.join("main.luau"), "return 1").unwrap();
        std::fs::write(dir.join("lib/util.luau"), "return 2").unwrap();
        std::fs::write(dir.join("lib/sub/init.luau"), "return 3").unwrap();
        dir
    }

    #[test]
    fn navigates_sibling_module() {
        let dir = tmp();
        let mut req = ScriptRequirer::new(dir.clone());
        req.reset(&format!("@{}/main.luau", dir.display())).unwrap();
        req.to_parent().unwrap();
        req.to_child("lib").unwrap();
        req.to_child("util").unwrap();
        assert!(req.has_module());
        assert!(req.cache_key().ends_with("lib/util.luau"));
    }

    #[test]
    fn resolves_init_files() {
        let dir = tmp();
        let mut req = ScriptRequirer::new(dir.clone());
        req.reset(&format!("@{}/main.luau", dir.display())).unwrap();
        req.to_parent().unwrap();
        req.to_child("lib").unwrap();
        req.to_child("sub").unwrap();
        assert!(req.has_module());
        assert!(req.cache_key().ends_with("lib/sub/init.luau"));
    }

    #[test]
    fn missing_module_names_the_path() {
        let dir = tmp();
        let mut req = ScriptRequirer::new(dir.clone());
        req.reset(&format!("@{}/main.luau", dir.display())).unwrap();
        req.to_parent().unwrap();
        req.to_child("lib").unwrap();
        // Navigating into a nonexistent module errors with the path named.
        let err = req.to_child("nope").unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("nope"), "{msg}");
        assert!(!req.has_module());
    }

    #[test]
    fn navigates_out_of_the_script_directory() {
        // Two sibling trees under one parent: workflow/main.luau requires
        // ../shared/helper, which walks out of the script root.
        let base = std::env::temp_dir().join(format!("ptah-req-cross-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("workflow")).unwrap();
        std::fs::create_dir_all(base.join("shared")).unwrap();
        std::fs::write(base.join("workflow/main.luau"), "return 1").unwrap();
        std::fs::write(base.join("shared/helper.luau"), "return 2").unwrap();

        let mut req = ScriptRequirer::new(base.join("workflow"));
        req.reset(&format!("@{}/workflow/main.luau", base.display()))
            .unwrap();
        // require("../shared/helper") from workflow/main.luau
        req.to_parent().unwrap(); // main.luau -> workflow/
        req.to_parent().unwrap(); // workflow/ -> base/ (outside the root)
        req.to_child("shared").unwrap();
        req.to_child("helper").unwrap();
        assert!(req.has_module());
        assert!(req.cache_key().ends_with("shared/helper.luau"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn absolute_require_strings_rejected() {
        let mut req = ScriptRequirer::new(tmp());
        let err = req.jump_to_alias("/etc/passwd").unwrap_err();
        assert!(matches!(err, NavigateError::Other(_)), "{err:?}");
    }

    // ------------------------------------------------------------------
    // Alias requires through .luaurc (end-to-end through a real Lua
    // instance - the same path `run` wires up).
    // ------------------------------------------------------------------

    fn lua_with_require_at(dir: &Path) -> Lua {
        let lua = Lua::new_with(mlua::StdLib::TABLE, mlua::LuaOptions::default()).unwrap();
        let require_fn = lua
            .create_require_function(ScriptRequirer::new(dir.to_path_buf()))
            .unwrap();
        lua.globals().set("require", require_fn).unwrap();
        lua
    }

    fn load_at(lua: &Lua, path: &Path, source: &str) -> mlua::Result<mlua::Value> {
        lua.load(source)
            .set_name(format!("@{}", path.display()))
            .eval()
    }

    fn alias_project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("ptah-alias-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".ptah/luau_packages")).unwrap();
        std::fs::create_dir_all(root.join(".ptah/workflows/x")).unwrap();
        std::fs::write(
            root.join(".luaurc"),
            r#"{"aliases": {"hello": "./.ptah/luau_packages/hello"}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join(".ptah/luau_packages/hello.luau"),
            "return { greet = 'hi from package' }\n",
        )
        .unwrap();
        root
    }

    #[test]
    fn alias_require_resolves_the_installed_package() {
        let root = alias_project("resolve");
        let script = root.join(".ptah/workflows/x/main.luau");
        std::fs::write(
            &script,
            "local a = require('@hello')\nlocal b = require('@hello')\nreturn a.greet .. tostring(a == b)\n",
        )
        .unwrap();
        let lua = lua_with_require_at(script.parent().unwrap());
        let out: String = load_at(&lua, &script, &std::fs::read_to_string(&script).unwrap())
            .unwrap()
            .to_string()
            .unwrap();
        assert_eq!(out, "hi from packagetrue", "loads and caches (same table)");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn alias_configuration_is_discovered_upward_from_the_requiring_file() {
        // The requiring file sits three levels below the config; the
        // alias resolves against the config's directory (the project
        // root), not the requiring file's.
        let root = alias_project("upward");
        let script = root.join(".ptah/workflows/x/deep.luau");
        std::fs::write(&script, "local m = require('@hello')\nreturn m.greet\n").unwrap();
        let lua = lua_with_require_at(script.parent().unwrap());
        let out: String = load_at(&lua, &script, &std::fs::read_to_string(&script).unwrap())
            .unwrap()
            .to_string()
            .unwrap();
        assert_eq!(out, "hi from package");

        // A deeper module file requiring through a relative hop also
        // resolves (the walk-up starts at the *requiring* file, not
        // the entry).
        let nested = root.join(".ptah/workflows/x/nested/mod.luau");
        std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::fs::write(&nested, "return require('@hello').greet\n").unwrap();
        let entry = root.join(".ptah/workflows/x/via_relative.luau");
        std::fs::write(&entry, "return require('./nested/mod')\n").unwrap();
        let lua = lua_with_require_at(entry.parent().unwrap());
        let out: String = load_at(&lua, &entry, &std::fs::read_to_string(&entry).unwrap())
            .unwrap()
            .to_string()
            .unwrap();
        assert_eq!(out, "hi from package");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn alias_require_can_continue_past_the_alias() {
        // `@hello/sub` resolves the alias then appends segments.
        let root = alias_project("segments");
        std::fs::create_dir_all(root.join(".ptah/luau_packages/hello/sub")).unwrap();
        std::fs::write(
            root.join(".ptah/luau_packages/hello/sub/init.luau"),
            "return 'sub module'\n",
        )
        .unwrap();
        let script = root.join(".ptah/workflows/x/main.luau");
        std::fs::write(&script, "return require('@hello/sub')\n").unwrap();
        let lua = lua_with_require_at(script.parent().unwrap());
        let out: String = load_at(&lua, &script, &std::fs::read_to_string(&script).unwrap())
            .unwrap()
            .to_string()
            .unwrap();
        assert_eq!(out, "sub module");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unknown_alias_rejected_naming_the_alias() {
        let root = alias_project("unknown");
        let script = root.join(".ptah/workflows/x/main.luau");
        std::fs::write(&script, "return require('@nope')\n").unwrap();
        let lua = lua_with_require_at(script.parent().unwrap());
        let err =
            load_at(&lua, &script, &std::fs::read_to_string(&script).unwrap()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("nope"), "names the alias: {msg}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn alias_require_without_any_configuration_rejected() {
        // No .luaurc anywhere up to the root: same rejection class.
        let root = alias_project("noconfig");
        std::fs::remove_file(root.join(".luaurc")).unwrap();
        let script = root.join(".ptah/workflows/x/main.luau");
        std::fs::write(&script, "return require('@hello')\n").unwrap();
        let lua = lua_with_require_at(script.parent().unwrap());
        let err =
            load_at(&lua, &script, &std::fs::read_to_string(&script).unwrap()).unwrap_err();
        assert!(err.to_string().contains("hello"), "{}", err);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn non_relative_non_alias_require_strings_still_rejected() {
        let root = alias_project("nonrelative");
        let script = root.join(".ptah/workflows/x/main.luau");
        std::fs::write(&script, "return require('shared/helper')\n").unwrap();
        let lua = lua_with_require_at(script.parent().unwrap());
        let err =
            load_at(&lua, &script, &std::fs::read_to_string(&script).unwrap()).unwrap_err();
        assert!(
            err.to_string().contains("must start with"),
            "states the allowed forms: {}",
            err
        );

        std::fs::write(&script, "return require('/etc/passwd')\n").unwrap();
        let lua = lua_with_require_at(script.parent().unwrap());
        let err =
            load_at(&lua, &script, &std::fs::read_to_string(&script).unwrap()).unwrap_err();
        assert!(err.to_string().contains("must start with"), "{}", err);
        let _ = std::fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // Pure helpers (the navigator's directory rules)
    // ------------------------------------------------------------------

    #[test]
    fn pure_helpers_normalize() {
        assert_eq!(
            normalize(Path::new("/base/dir/./lib/../lib/x")),
            Path::new("/base/dir/lib/x")
        );
        // `..` at the root pops.
        assert_eq!(normalize(Path::new("/..")), Path::new("/"));
    }
}
