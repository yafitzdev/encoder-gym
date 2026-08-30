use std::process::{Command, Output};

use serde_json::Value;

pub fn run<'a>(database_url: &str, arguments: impl IntoIterator<Item = &'a str>) -> Output {
    Command::new(env!("CARGO_BIN_EXE_synth"))
        .args(["--database-url", database_url, "--output", "json"])
        .args(arguments)
        .output()
        .expect("CLI starts")
}

pub fn run_json<'a>(database_url: &str, arguments: impl IntoIterator<Item = &'a str>) -> Value {
    let output = run(database_url, arguments);
    assert!(
        output.status.success(),
        "CLI failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout was not one JSON value: {error}\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

pub fn run_text<'a>(database_url: &str, arguments: impl IntoIterator<Item = &'a str>) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_synth"))
        .args(["--database-url", database_url, "--output", "human"])
        .args(arguments)
        .output()
        .expect("CLI starts");
    assert!(
        output.status.success(),
        "CLI failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("human output is UTF-8")
}
