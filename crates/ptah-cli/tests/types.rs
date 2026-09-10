//! Type-definition sync guard: the probe fixture
//! (`tests/fixtures/types_probe.luau`, `--!strict`) exercises every member,
//! method, and field `.ptah/ptah.d.luau` promises, against the mock agent.
//! If the runtime drops or renames anything the definitions document, this
//! test fails. (Static analysis of the fixture lives in the nix
//! `ptah-analyze` check; `ptah types` output sync lives in cli.rs.)

use std::path::PathBuf;

mod common;

fn mock_bin() -> &'static str {
    env!("CARGO_BIN_EXE_mock-agent")
}

#[test]
fn type_definitions_probe() {
    let dir = std::env::temp_dir().join(format!("ptah-types-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".ptah")).unwrap();

    // `demo`: normal turns with known usage counts. `demo-hang`: prompts
    // never complete on their own (cancel path). `demo-config`: advertises
    // a select `model` option and a boolean `fast` option, and echoes the
    // current model value in each reply (config-options surface).
    let config = format!(
        r#"[agents.demo]
command = "{mock}"
args = []

[agents.demo.env]
MOCK_USAGE = "5,10,2,3"

[agents.demo-hang]
command = "{mock}"
args = []

[agents.demo-hang.env]
MOCK_HANG = "1"

[agents.demo-config]
command = "{mock}"
args = []

[agents.demo-config.env]
MOCK_CONFIG_OPTIONS = '{config_options}'
MOCK_CONFIG_ECHO = "model"
"#,
        mock = mock_bin(),
        config_options = r#"[{"id":"model","name":"Model","category":"model","type":"select","currentValue":"opus","options":[{"value":"opus","name":"Opus"},{"value":"haiku","name":"Haiku"}]},{"id":"fast","name":"Fast mode","type":"boolean","currentValue":false}]"#
    );
    std::fs::write(dir.join(".ptah").join("config.toml"), config).unwrap();

    // The fixture carries a @MOCK_BIN@ placeholder so it stays a valid
    // strict script for the static-analysis gate; substitute at run time.
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/types_probe.luau");
    let script = std::fs::read_to_string(&fixture)
        .unwrap()
        .replace("@MOCK_BIN@", mock_bin());
    let script_path = dir.join("main.luau");
    std::fs::write(&script_path, script).unwrap();

    // The probe asks twice (ptah.ask coverage): drive the real stdin
    // provider over a pipe — the explicit PTAH_ASK makes it legal without
    // a terminal — answering `go` then `/abort`, exactly the two result
    // arms the fixture asserts. If `ask` disappeared from the runtime the
    // `ptah.ask` call fails the probe; if it disappeared from the defs,
    // the nix ptah-analyze gate fails instead. PTAH_ENV_* exercise
    // os.getenv's snapshot inside the probe: set, unset (absent here),
    // and set-to-empty.
    let mut run = common::PipedRun::spawn_env(
        &dir,
        &script_path,
        &[],
        &[
            ("PTAH_ASK", "stdin"),
            ("PTAH_ENV_PROBE", "x"),
            ("PTAH_ENV_EMPTY", ""),
        ],
    );
    run.wait_for("ask 1 main.luau: Proceed with the probe?");
    run.write_line("go");
    run.wait_for("ask 2 main.luau: Abort this one?");
    run.write_line("/abort");
    let (code, stdout, stderr) = run.finish();
    assert_eq!(
        code, 0,
        "type-definitions probe failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("probe complete"), "{stdout}");
}
