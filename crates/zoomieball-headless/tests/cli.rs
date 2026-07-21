//! Public headless CLI contract tests.

use std::process::Command;

#[test]
fn hash_stream_names_all_layers_and_the_fixed_schedule() {
    let output = Command::new(env!("CARGO_BIN_EXE_zoomieball-headless"))
        .args(["10", "1", "--hashes"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("physics="));
    assert!(stdout.contains("controller="));
    assert!(stdout.contains("learning="));
    assert!(stdout.contains("pipeline="));
    assert!(stdout.contains("schedule=60/15/120Hz"));
    assert!(!stdout.contains("world="));
    assert!(!stdout.contains("combined="));
}
