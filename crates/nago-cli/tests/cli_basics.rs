use assert_cmd::Command;
use predicates::prelude::*;

fn nago() -> Command {
    Command::cargo_bin("nago").unwrap()
}

#[test]
fn opcao_desconhecida_sai_com_codigo_2() {
    nago()
        .arg("--opcao-que-nao-existe")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Usage"));
}

#[test]
fn subcomando_desconhecido_sai_com_codigo_2() {
    nago().arg("explodir").assert().code(2);
}

#[test]
fn ajuda_lista_os_subcomandos() {
    nago().arg("--help").assert().success().stdout(
        predicate::str::contains("info")
            .and(predicate::str::contains("cut"))
            .and(predicate::str::contains("convert"))
            .and(predicate::str::contains("compress"))
            .and(predicate::str::contains("doctor")),
    );
}

#[test]
fn cut_rejeita_to_e_duration_juntos() {
    nago()
        .args(["cut", "a.mp4", "--to", "10", "--duration", "5"])
        .assert()
        .code(2);
}

#[test]
fn sem_ffmpeg_no_path_sai_com_codigo_1_e_dica_de_instalacao() {
    nago()
        .args(["info", "qualquer.mp4"])
        .env("PATH", "")
        .env_remove("NAGO_FFMPEG")
        .env_remove("NAGO_FFPROBE")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("ffmpeg").and(predicate::str::contains("instale")));
}

#[test]
fn nago_ffmpeg_invalido_sai_com_codigo_1() {
    nago()
        .args(["doctor"])
        .env("NAGO_FFMPEG", "/nao/existe/ffmpeg")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("NAGO_FFMPEG"));
}

#[cfg(unix)]
#[test]
fn ffmpeg_antigo_informa_versao_minima_com_codigo_1() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("ffmpeg");
    std::fs::write(
        &fake,
        "#!/bin/sh\necho 'ffmpeg version 5.1.4 Copyright (c) 2000-2023'\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    nago()
        .arg("doctor")
        .env("NAGO_FFMPEG", &fake)
        .env("NAGO_FFPROBE", &fake)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("5.1.4").and(predicate::str::contains("6.0.0")));
}

/// `nago` sem subcomando fora de um terminal (ex.: `nago | cat`) mostra a ajuda e sai
/// com 2, sem abrir a TUI.
#[test]
fn sem_subcomando_fora_de_tty_mostra_ajuda_com_codigo_2() {
    nago()
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Usage").and(predicate::str::contains("tui")));
}

#[test]
fn tui_fora_de_tty_e_recusada_com_codigo_2() {
    nago()
        .args(["tui"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("precisa de um terminal"));
}

#[test]
fn tui_com_json_e_erro_de_uso() {
    nago()
        .args(["--json", "tui"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--json"));
}
