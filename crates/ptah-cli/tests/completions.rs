//! `ptah completions` e2e: the real binary emits per-shell completion
//! scripts generated from its own command tree — visible subcommands
//! present, the hidden `__bridge` absent — and rejects unknown shells
//! as usage errors. Markers pin the per-shell registration contract,
//! not golden files (a format change that keeps the contract passes).

use std::process::Command;

fn ptah_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ptah")
}

/// (shell, plausibility markers clap_complete emits for that dialect).
/// Bash's `complete -F` satisfies the spec's `complete` marker more
/// specifically than the bare word; elvish's stable anchor is its
/// `arg-completer` hook registration.
const SHELLS: &[(&str, &[&str])] = &[
    ("bash", &["complete -F"]),
    ("zsh", &["#compdef"]),
    ("fish", &["complete", "__fish"]),
    ("elvish", &["arg-completer"]),
    ("powershell", &["Register-ArgumentCompleter"]),
];

#[test]
fn every_shell_emits_a_plausible_script() {
    for (shell, markers) in SHELLS {
        let out = Command::new(ptah_bin())
            .args(["completions", shell])
            .output()
            .unwrap();
        assert!(out.status.success(), "{shell}: {out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(!stdout.trim().is_empty(), "{shell}: empty script");
        for marker in *markers {
            assert!(
                stdout.contains(marker),
                "{shell}: missing marker {marker:?}:\n{stdout}"
            );
        }
        assert!(
            out.stderr.is_empty(),
            "{shell}: completions print nothing but the script"
        );
    }
}

#[test]
fn visible_subcommands_appear_and_bridge_does_not() {
    for (shell, _) in SHELLS {
        let out = Command::new(ptah_bin())
            .args(["completions", shell])
            .output()
            .unwrap();
        assert!(out.status.success(), "{shell}: {out:?}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        for sub in ["run", "check", "types", "completions", "init", "package"] {
            assert!(
                stdout.contains(sub),
                "{shell}: visible subcommand {sub} missing:\n{stdout}"
            );
        }
        assert!(
            !stdout.contains("__bridge"),
            "{shell}: hidden __bridge leaked into the script:\n{stdout}"
        );
    }
}

#[test]
fn unknown_shell_is_a_usage_error() {
    let out = Command::new(ptah_bin())
        .args(["completions", "tcsh"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    // clap prints the invalid-value error with the accepted shells
    // (its usage guidance for this error kind) and no Usage: block.
    assert!(
        stderr.contains("error:"),
        "expected an error on stderr: {stderr}"
    );
    assert!(
        stderr.contains("possible values"),
        "expected the accepted shells on stderr: {stderr}"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).is_empty(),
        "no partial script on stdout"
    );
}

#[test]
fn completions_need_no_registry_and_touch_nothing() {
    // No registry (empty HOME, empty cwd), no `.ptah` dir: the command
    // still succeeds and creates no files — generation is pure.
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let out = Command::new(ptah_bin())
        .args(["completions", "fish"])
        .current_dir(dir.path())
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(
        entries,
        vec![home.canonicalize().unwrap()],
        "completions must create no files or directories"
    );
}
