use assert_cmd::Command;
use nago_testkit::{Fixture, fixture};
use predicates::prelude::*;

fn nago() -> Command {
    Command::cargo_bin("nago").unwrap()
}

#[test]
fn info_mostra_todos_os_streams() {
    nago()
        .arg("info")
        .arg(fixture(Fixture::MkvMulti))
        .assert()
        .success()
        .stdout(
            predicate::str::contains("h264")
                .and(predicate::str::contains("640x360"))
                .and(predicate::str::contains("áudio").count(2))
                .and(predicate::str::contains("legenda").count(2))
                .and(predicate::str::contains("00:00:06.0"))
                .and(predicate::str::contains("kB")),
        );
}

#[test]
fn info_json_e_apenas_json_valido_com_campos_do_esquema() {
    let out = nago()
        .args(["info", "--json"])
        .arg(fixture(Fixture::H264Aac360p))
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout é só JSON");
    for key in [
        "container",
        "duration_secs",
        "size_bytes",
        "bit_rate",
        "streams",
    ] {
        assert!(v.get(key).is_some(), "falta {key}");
    }
    insta::assert_json_snapshot!("info_h264_aac_360p", v, {
        ".path" => "[caminho]",
        ".size_bytes" => "[tamanho]",
        ".bit_rate" => "[bitrate]",
        ".streams[].bit_rate" => "[bitrate]",
    });
}

#[test]
fn info_hdr_marca_hdr10() {
    nago()
        .arg("info")
        .arg(fixture(Fixture::HevcHdr10))
        .assert()
        .success()
        .stdout(predicate::str::contains("[HDR10]").and(predicate::str::contains("10-bit")));
}

#[test]
fn info_arquivo_inexistente_sai_com_1() {
    nago()
        .args(["info", "nao-existe.mp4"])
        .assert()
        .code(1)
        .stderr(
            predicate::str::contains("não foi encontrado")
                .or(predicate::str::contains("não encontrado")),
        );
}

#[test]
fn info_arquivo_que_nao_e_midia_sai_com_1() {
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("notas.txt");
    std::fs::write(&txt, "texto qualquer\n").unwrap();
    nago()
        .arg("info")
        .arg(&txt)
        .assert()
        .code(1)
        .stderr(predicate::str::contains("não é uma mídia suportada"));
}

/// Texto humano do `nago info` (a primeira linha é o caminho, que varia).
#[test]
fn info_texto_humano() {
    let out = nago()
        .arg("info")
        .arg(fixture(Fixture::MkvMulti))
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let body: Vec<&str> = text.lines().skip(1).collect();
    insta::assert_snapshot!(body.join("\n"));
}
