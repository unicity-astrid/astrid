use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn resolver_helper_updates_request_and_writes_inbox_note() {
    let requests_dir = temp_dir("bridge_resolver_requests");
    let reviewed_dir = requests_dir.join("reviewed");
    fs::create_dir_all(&reviewed_dir).unwrap();
    let inbox_dir = temp_dir("bridge_resolver_inbox");
    let request_path = requests_dir.join("agency_code_change_1.json");
    fs::write(
        &request_path,
        r#"{
  "id": "agency_code_change_1",
  "timestamp": "1774635155",
  "source_journal_path": "/tmp/!astrid_1774635155.txt",
  "request_kind": "code_change",
  "title": "Give me a governed path",
  "felt_need": "I want longing to become action.",
  "why_now": "The constraint feels urgent.",
  "status": "pending",
  "acceptance_signals": ["Astrid gets a concrete note."],
  "target_paths": ["/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs"],
  "target_symbols": ["Mode::Evolve"],
  "requested_behavior": "Add an EVOLVE queue.",
  "constraints": ["Draft only."],
  "draft_patch": null
}
"#,
    )
    .unwrap();

    let script = "/Users/v/other/astrid/capsules/consciousness-bridge/resolve_agency_request.py";
    let output = Command::new("python3")
        .arg(script)
        .arg("--request")
        .arg(&request_path)
        .arg("--status")
        .arg("completed")
        .arg("--summary")
        .arg("Added the EVOLVE queue and Claude handoff.")
        .arg("--file")
        .arg("/Users/v/other/astrid/capsules/consciousness-bridge/src/autonomous.rs")
        .arg("--inbox-dir")
        .arg(&inbox_dir)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "resolver failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let note_path = inbox_dir.join("agency_status_agency_code_change_1.txt");
    assert!(note_path.exists());
    assert!(reviewed_dir.join("agency_code_change_1.json").exists());
    assert!(!request_path.exists());

    let note = fs::read_to_string(note_path).unwrap();
    assert!(note.contains("Added the EVOLVE queue and Claude handoff."));

    let _ = fs::remove_dir_all(&requests_dir);
    let _ = fs::remove_dir_all(&inbox_dir);
}
