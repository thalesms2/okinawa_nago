use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn doctor_lista_versao_encoders_e_vmaf() {
    let cache = tempfile::tempdir().unwrap();
    Command::cargo_bin("nago")
        .unwrap()
        .arg("doctor")
        .env("NAGO_CACHE_DIR", cache.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("ffmpeg")
                .and(predicate::str::contains("libvmaf"))
                .and(predicate::str::contains("libsvtav1"))
                .and(predicate::str::contains("libx265"))
                .and(predicate::str::contains("libx264"))
                .and(predicate::str::contains("av1_nvenc")),
        );
    assert!(
        cache.path().join("caps.json").exists(),
        "doctor atualiza o cache"
    );
}

#[test]
fn doctor_json() {
    let cache = tempfile::tempdir().unwrap();
    let out = Command::cargo_bin("nago")
        .unwrap()
        .args(["doctor", "--json"])
        .env("NAGO_CACHE_DIR", cache.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["libvmaf"].is_boolean());
    let encs = v["encoders"].as_array().unwrap();
    assert_eq!(encs.len(), 12);
    let svt = encs.iter().find(|e| e["name"] == "libsvtav1").unwrap();
    assert_eq!(svt["status"]["status"], "available");
}
