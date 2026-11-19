//! Spec `video-compression`.

use assert_cmd::Command;
use nago_testkit::{Fixture, ffprobe_json, fixture_in};
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
fn hdr10_para_av1_preserva_transferencia_primarias_e_10bit() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::HevcHdr10, dir.path());
    nago()
        .args(["compress", "--force", "--codec", "av1"])
        .arg(&input)
        .assert()
        .success();
    let out = dir.path().join("hevc_hdr10.compressed.mkv");
    let v = &ffprobe_json(&out)["streams"][0];
    assert_eq!(v["codec_name"], "av1");
    assert_eq!(v["pix_fmt"], "yuv420p10le");
    assert_eq!(v["color_transfer"], "smpte2084");
    assert_eq!(v["color_primaries"], "bt2020");
    // e o nago lê de volta como HDR10 com mastering display
    let info = nago().args(["info", "--json"]).arg(&out).output().unwrap();
    let info: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    let video = &info["streams"][0]["video"];
    assert_eq!(video["hdr"], "HDR10");
    assert!(
        (video["mastering_display"]["max_luminance"]
            .as_f64()
            .unwrap()
            - 1000.0)
            .abs()
            < 1.0
    );
    assert_eq!(video["content_light"]["max_cll"], 1000);
}

#[test]
fn hdr10_para_hevc_preserva_metadados() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::HevcHdr10, dir.path());
    nago()
        .args([
            "compress",
            "--force",
            "--codec",
            "hevc",
            "--quality",
            "small",
        ])
        .arg(&input)
        .assert()
        .success();
    let info = nago()
        .args(["info", "--json"])
        .arg(dir.path().join("hevc_hdr10.compressed.mkv"))
        .output()
        .unwrap();
    let info: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
    assert_eq!(info["streams"][0]["video"]["hdr"], "HDR10");
    assert_eq!(
        info["streams"][0]["video"]["content_light"]["max_fall"],
        400
    );
}

#[test]
fn hdr_com_h264_e_recusado_com_codigo_2() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::HevcHdr10, dir.path());
    nago()
        .args(["compress", "--codec", "h264"])
        .arg(&input)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("HDR").and(predicate::str::contains("--codec av1")));
    assert_eq!(files_in(dir.path()), ["hevc_hdr10.mkv"]);
}

#[test]
fn saida_maior_que_a_entrada_e_descartada_com_codigo_3() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::Av1Low, dir.path());
    nago()
        .args(["compress", "--quality", "visually-lossless"])
        .arg(&input)
        .assert()
        .code(3)
        .stderr(predicate::str::contains("não reduziria o tamanho"));
    assert_eq!(
        files_in(dir.path()),
        ["av1_low.mkv"],
        "nenhuma saída deve permanecer"
    );
}

#[test]
fn force_mantem_saida_maior() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::Av1Low, dir.path());
    nago()
        .args(["compress", "--quality", "visually-lossless", "--force"])
        .arg(&input)
        .assert()
        .success()
        .stdout(predicate::str::contains("(+"));
    assert!(dir.path().join("av1_low.compressed.mkv").exists());
}

#[test]
fn verify_inclui_vmaf_no_relatorio() {
    if !nago_testkit::has_libvmaf() {
        eprintln!("pulado: ffmpeg sem libvmaf");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    // Saída MKV de propósito: timestamps em ms não podem desalinhar a comparação.
    let out = nago()
        .args(["compress", "--verify", "--force", "--json", "-o"])
        .arg(dir.path().join("saida.mkv"))
        .arg(&input)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mean = v["vmaf_mean"].as_f64().expect("vmaf_mean");
    let p5 = v["vmaf_p5"].as_f64().expect("vmaf_p5");
    assert!(
        mean > 50.0 && mean <= 100.0 && p5 <= mean + 1e-9,
        "{mean} {p5}"
    );
    // sem frames desalinhados, o pior trecho fica perto da média
    assert!(
        p5 > mean - 5.0,
        "p5 {p5} muito abaixo da média {mean}: frames desalinhados?"
    );
    assert_eq!(v["vmaf_target"], 93.0);
}

#[test]
fn audio_e_legendas_copiados_por_padrao() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::MkvMulti, dir.path());
    let out = nago()
        .args(["compress", "--force", "--quality", "small", "--json"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let actions: Vec<&str> = v["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["transcode", "copy", "copy", "copy", "copy"]);
    let probe = ffprobe_json(&dir.path().join("multi_tracks.compressed.mkv"));
    let codecs: Vec<&str> = probe["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["codec_name"].as_str().unwrap())
        .collect();
    assert_eq!(codecs, ["av1", "aac", "aac", "subrip", "subrip"]);
}

#[test]
fn audio_bitrate_recodifica_para_opus() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    nago()
        .args([
            "compress",
            "--force",
            "--quality",
            "small",
            "--audio-bitrate",
            "96",
            "-o",
        ])
        .arg(dir.path().join("saida.mkv"))
        .arg(&input)
        .assert()
        .success();
    let probe = ffprobe_json(&dir.path().join("saida.mkv"));
    assert_eq!(probe["streams"][1]["codec_name"], "opus");
}

#[test]
fn mantem_resolucao_e_fps() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    nago()
        .args([
            "compress",
            "--force",
            "--codec",
            "hevc",
            "--quality",
            "small",
        ])
        .arg(&input)
        .assert()
        .success();
    let probe = ffprobe_json(&dir.path().join("h264_aac_360p.compressed.mp4"));
    let v = &probe["streams"][0];
    assert_eq!(v["codec_name"], "hevc");
    assert_eq!(v["codec_tag_string"], "hvc1");
    assert_eq!(
        (v["width"].as_u64(), v["height"].as_u64()),
        (Some(640), Some(360))
    );
    assert_eq!(v["avg_frame_rate"], "30/1");
}

#[cfg(unix)]
#[test]
fn verify_sem_libvmaf_falha_antes_de_codificar() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    let real = String::from_utf8(
        std::process::Command::new("sh")
            .args(["-c", "command -v ffmpeg"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let fake = dir.path().join("ffmpeg-sem-vmaf");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n*-filters*) {} \"$@\" | grep -v libvmaf;;\n*-progress*) echo 'NÃO DEVIA CODIFICAR' >&2; exit 1;;\n*) exec {} \"$@\";;\nesac\n",
            real.trim(),
            real.trim()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cache = tempfile::tempdir().unwrap();
    nago()
        .env("NAGO_FFMPEG", &fake)
        .env("NAGO_CACHE_DIR", cache.path())
        .args(["compress", "--verify"])
        .arg(&input)
        .assert()
        .code(1)
        .stderr(
            predicate::str::contains("libvmaf").and(predicate::str::contains("NÃO DEVIA").not()),
        );
}

#[test]
fn avi_de_entrada_vira_compressed_mkv() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("antigo.avi");
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=25:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=2",
            "-c:v",
            "mpeg4",
            "-q:v",
            "2",
            "-c:a",
            "libmp3lame",
        ])
        .arg(&input)
        .status()
        .unwrap();
    assert!(ok.success());
    nago().arg("compress").arg(&input).assert().success();
    let out = dir.path().join("antigo.compressed.mkv");
    assert_eq!(nago_testkit::stream_codecs(&out), ["av1", "mp3"]);
}

#[test]
fn perfil_padrao_aparece_no_relatorio() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    nago()
        .args(["compress", "--force"])
        .arg(&input)
        .assert()
        .success()
        .stdout(predicate::str::contains("perfil high"));
}

/// Máquina de referência (RTX 4060 Ti): `cargo test -- --ignored`.
#[test]
#[ignore = "requer GPU NVIDIA com NVENC AV1"]
fn gpu_usa_av1_nvenc() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    let out = nago()
        .args(["compress", "--gpu", "--force", "--json"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["encoder"], "av1_nvenc");
}
