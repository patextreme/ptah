//! Type-definition sync guard: the probe fixture
//! (`tests/fixtures/types_probe.luau`, `--!strict`) exercises every member,
//! method, and field `.ptah/ptah.d.luau` promises, against the mock agent.
//! If the runtime drops or renames anything the definitions document, this
//! test fails. (Static analysis of the fixture lives in the nix
//! `ptah-analyze` check; `ptah types` output sync lives in cli.rs.)

use std::path::PathBuf;
use std::process::Command;

fn ptah_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ptah")
}

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

    let output = Command::new(ptah_bin())
        .arg("run")
        .arg(&script_path)
        .current_dir(&dir)
        // PTAH_ENV_* exercise os.getenv's snapshot inside the probe:
        // set, unset (absent from this Command), and set-to-empty.
        .env("PTAH_ENV_PROBE", "x")
        .env("PTAH_ENV_EMPTY", "")
        .output()
        .expect("run ptah");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "type-definitions probe failed:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("probe complete"), "{stdout}");
}
