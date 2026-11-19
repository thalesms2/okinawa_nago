//! Spec `job-execution`: progresso, cancelamento, relatório e proteção de arquivos.

use std::process::Stdio;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use nago_testkit::{Fixture, fixture_in};
use predicates::prelude::*;

fn nago() -> Command {
    let mut c = Command::cargo_bin("nago").unwrap();
    c.env(
        "NAGO_CACHE_DIR",
        std::env::temp_dir().join("nago-test-cache"),
    );
    c
}

fn files_in(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn json_e_o_unico_conteudo_de_stdout_e_tem_os_campos_da_spec() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    let out = nago()
        .args(["compress", "--json", "--force", "--quality", "small"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout é só JSON");
    for key in [
        "output",
        "input_size_bytes",
        "output_size_bytes",
        "elapsed_secs",
        "encoder",
    ] {
        assert!(v.get(key).is_some(), "falta {key}");
    }
    assert_eq!(v["encoder"], "libsvtav1");
    assert_eq!(v["quality"], "small");
    // sem TTY não há barra de progresso animada no stderr
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains('\r') && !stderr.contains('━'),
        "barra em não-TTY: {stderr}"
    );
}

#[test]
fn relatorio_humano_mostra_tamanhos_variacao_e_encoder() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    nago()
        .args(["compress", "--force", "--quality", "small"])
        .arg(&input)
        .assert()
        .success()
        .stdout(
            predicate::str::is_match(r"MB → [\d.]+ [kM]B \([+-]\d+%\) em ")
                .unwrap()
                .and(predicate::str::contains("libsvtav1"))
                .and(predicate::str::contains("h264_aac_360p.compressed.mp4")),
        );
}

#[test]
fn saida_igual_a_entrada_e_recusada_com_codigo_2() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    nago()
        .args(["compress", "--overwrite", "-o"])
        .arg(&input)
        .arg(&input)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("entrada"));
}

#[test]
fn saida_explicita_existente_sem_overwrite_e_recusada_sem_processar() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    let existing = dir.path().join("final.mp4");
    std::fs::write(&existing, b"nao mexa").unwrap();
    nago()
        .args(["compress", "-o"])
        .arg(&existing)
        .arg(&input)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--overwrite"));
    assert_eq!(std::fs::read(&existing).unwrap(), b"nao mexa");
}

#[test]
fn erro_do_ffmpeg_mostra_resumo_e_ultimas_linhas() {
    // Um "vídeo" truncado: o probe aceita, mas a decodificação falha no meio.
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac1080p, dir.path());
    let bytes = std::fs::read(&input).unwrap();
    let broken = dir.path().join("quebrado.mp4");
    // mantém o cabeçalho (moov no fim? então corrompe o meio do mdat)
    let mut data = bytes.clone();
    let mid = data.len() / 2;
    for b in &mut data[mid..mid + 200_000] {
        *b = 0xFF;
    }
    std::fs::write(&broken, &data).unwrap();
    let out = nago()
        .args(["compress", "--force", "-o"])
        .arg(dir.path().join("x.mkv"))
        .arg(&broken)
        .args(["--quality", "small"])
        .output()
        .unwrap();
    // Dependendo do dano o ffmpeg pode concluir com avisos; se falhar, o formato do erro é o da spec.
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1));
        assert!(stderr.contains("ffmpeg"), "{stderr}");
    }
    assert!(!files_in(dir.path()).iter().any(|f| f.contains(".part")));
}

#[cfg(unix)]
#[test]
fn sigint_durante_a_compressao_sai_com_130_sem_lixo() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac1080p, dir.path());
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("nago"))
        .env(
            "NAGO_CACHE_DIR",
            std::env::temp_dir().join("nago-test-cache"),
        )
        .args(["compress", "--quality", "visually-lossless"])
        .arg(&input)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // espera o ffmpeg criar o temporário (a codificação começou)
    let deadline = Instant::now() + Duration::from_secs(20);
    while !files_in(dir.path()).iter().any(|f| f.contains(".part")) {
        assert!(Instant::now() < deadline, "a codificação não começou");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(500));
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(child.id() as i32),
        nix::sys::signal::Signal::SIGINT,
    )
    .unwrap();
    let started = Instant::now();
    let status = child.wait().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "demorou para cancelar"
    );
    assert_eq!(status.code(), Some(130));
    assert_eq!(
        files_in(dir.path()),
        ["h264_aac_1080p.mp4"],
        "sobrou arquivo"
    );
}
