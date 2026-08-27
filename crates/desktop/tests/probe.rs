use std::process::Command;

#[test]
fn probe_prints_json_without_starting_gpui() {
    let output = Command::new(env!("CARGO_BIN_EXE_RapidCap"))
        .arg("--probe")
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["app_id"], "com.inspire.rapidcap");
    assert_eq!(value["hotkeys"].as_array().unwrap().len(), 5);
}
