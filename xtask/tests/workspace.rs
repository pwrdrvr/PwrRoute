use std::{env, fs, process::Command, time::SystemTime};

#[test]
fn cargo_runtime_manifest_selects_the_active_workspace() {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = env::temp_dir().join(format!("pwrroute-xtask-{}-{nonce}", std::process::id()));
    let manifest_dir = root.join("xtask");
    fs::create_dir_all(&manifest_dir).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace.package]\nversion = \"9.8.7\"\n",
    )
    .unwrap();
    fs::write(
        root.join("CHANGELOG.md"),
        "# Changelog\n\n## v9.8.7\n\n- Active workspace.\n",
    )
    .unwrap();

    // Cargo can reuse an xtask compiled in another worktree when the target
    // cache is shared. It sets this runtime path for the invoking package.
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["release-check", "--tag", "v9.8.7"])
        .env("CARGO_MANIFEST_DIR", &manifest_dir)
        .current_dir(&root)
        .output()
        .unwrap();
    fs::remove_dir_all(&root).unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "release metadata agrees for v9.8.7\n"
    );
}
