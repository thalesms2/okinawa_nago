//! Spec `container-conversion`.

use std::path::Path;

use assert_cmd::Command;
use nago_testkit::{Fixture, ffprobe_json, fixture_in, stream_codecs};
use predicates::prelude::*;

fn nago() -> Command {
    let mut c = Command::cargo_bin("nago").unwrap();
    c.env(
        "NAGO_CACHE_DIR",
        std::env::temp_dir().join("nago-test-cache"),
    );
    c
}

/// Hash dos pacotes de cada stream (só dados, sem timestamps).
fn streamhash(path: &Path, spec: &str) -> String {
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-map",
            spec,
            "-c",
            "copy",
            "-f",
            "streamhash",
            "-hash",
            "md5",
            "-",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| l.splitn(3, ',').nth(2).unwrap_or("").to_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Ordem dos átomos de topo de um arquivo ISO-BMFF.
fn top_level_atoms(path: &Path) -> Vec<String> {
    let data = std::fs::read(path).unwrap();
    let mut atoms = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let mut size = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as u64;
        let kind = String::from_utf8_lossy(&data[pos + 4..pos + 8]).into_owned();
        if size == 1 {
            size = u64::from_be_bytes(data[pos + 8..pos + 16].try_into().unwrap());
        } else if size == 0 {
            size = (data.len() - pos) as u64;
        }
        atoms.push(kind);
        pos += size as usize;
    }
    atoms
}

fn json_of(out: &std::process::Output) -> serde_json::Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn mkv_h264_aac_para_mp4_copia_streams_bit_a_bit() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::MkvMulti, dir.path());
    let out = nago()
        .args(["convert", "--to", "mp4", "--json"])
        .arg(&input)
        .output()
        .unwrap();
    let report = json_of(&out);
    let output = dir.path().join("multi_tracks.mp4");
    assert_eq!(report["output"].as_str().unwrap(), output.to_str().unwrap());
    let actions: Vec<&str> = report["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["action"].as_str().unwrap())
        .collect();
    assert_eq!(actions, ["copy", "copy", "copy", "transcode", "transcode"]);
    assert!(report["encoder"].is_null());
    for spec in ["0:v:0", "0:a:0", "0:a:1"] {
        assert_eq!(
            streamhash(&input, spec),
            streamhash(&output, spec),
            "{spec} difere"
        );
    }
    let atoms = top_level_atoms(&output);
    let moov = atoms.iter().position(|a| a == "moov").unwrap();
    let mdat = atoms.iter().position(|a| a == "mdat").unwrap();
    assert!(moov < mdat, "moov depois de mdat: {atoms:?}");
}

#[test]
fn formato_inferido_pela_extensao_de_saida() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    let output = dir.path().join("video.mkv");
    nago()
        .arg("convert")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .assert()
        .success();
    assert_eq!(
        ffprobe_json(&output)["format"]["format_name"],
        "matroska,webm"
    );
}

#[test]
fn formato_nao_suportado_lista_os_suportados_com_codigo_2() {
    nago()
        .args(["convert", "x.mkv", "--to", "avi"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("mp4").and(predicate::str::contains("webm")));
}

#[test]
fn sem_formato_e_sem_saida_e_erro_de_uso() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    nago()
        .arg("convert")
        .arg(&input)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--to"));
}

#[test]
fn mov_com_pcm_para_mp4_copia_video_e_recodifica_audio() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::MovPcm, dir.path());
    let out = nago()
        .args(["convert", "--to", "mp4", "--json"])
        .arg(&input)
        .output()
        .unwrap();
    let report = json_of(&out);
    assert_eq!(report["streams"][0]["action"], "copy");
    assert_eq!(report["streams"][1]["action"], "transcode");
    assert!(
        report["streams"][1]["reason"]
            .as_str()
            .unwrap()
            .contains("pcm_s16le")
    );
    let output = dir.path().join("h264_pcm.mp4");
    assert_eq!(stream_codecs(&output), ["h264", "aac"]);
    assert_eq!(streamhash(&input, "0:v:0"), streamhash(&output, "0:v:0"));
    // relatório humano informa a recodificação
    let dir2 = tempfile::tempdir().unwrap();
    let input2 = fixture_in(Fixture::MovPcm, dir2.path());
    nago()
        .args(["convert", "--to", "mp4"])
        .arg(&input2)
        .assert()
        .success()
        .stdout(predicate::str::contains("pcm_s16le → aac"));
}

#[test]
fn mp4_para_webm_vira_av1_e_opus() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    let out = nago()
        .args(["convert", "--to", "webm", "--json"])
        .arg(&input)
        .output()
        .unwrap();
    let report = json_of(&out);
    assert_eq!(report["encoder"], "libsvtav1");
    assert_eq!(report["quality"], "high");
    assert_eq!(
        stream_codecs(&dir.path().join("h264_aac_360p.webm")),
        ["av1", "opus"]
    );
}

#[test]
fn legendas_srt_viram_mov_text_no_mp4_e_webvtt_no_webm() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::MkvMulti, dir.path());
    nago()
        .args(["convert", "--to", "mp4"])
        .arg(&input)
        .assert()
        .success();
    let mp4 = ffprobe_json(&dir.path().join("multi_tracks.mp4"));
    let subs: Vec<(&str, &str)> = mp4["streams"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["codec_type"] == "subtitle")
        .map(|s| {
            (
                s["codec_name"].as_str().unwrap(),
                s["tags"]["language"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(subs, [("mov_text", "por"), ("mov_text", "eng")]);

    nago()
        .args(["convert", "--to", "webm"])
        .arg(&input)
        .assert()
        .success();
    let codecs = stream_codecs(&dir.path().join("multi_tracks.webm"));
    assert_eq!(codecs, ["av1", "opus", "opus", "webvtt", "webvtt"]);
}

#[test]
fn extrai_aac_para_m4a_sem_recodificar() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    let out = nago()
        .args(["convert", "--to", "m4a", "--json"])
        .arg(&input)
        .output()
        .unwrap();
    let report = json_of(&out);
    assert_eq!(report["streams"].as_array().unwrap().len(), 1);
    assert_eq!(report["streams"][0]["action"], "copy");
    let output = dir.path().join("h264_aac_360p.m4a");
    assert_eq!(stream_codecs(&output), ["aac"]);
    assert_eq!(streamhash(&input, "0:a:0"), streamhash(&output, "0:a:0"));
}

#[test]
fn audio_track_escolhe_a_faixa_e_formatos_de_audio_funcionam() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::MkvMulti, dir.path());
    for (fmt, codec) in [("mp3", "mp3"), ("opus", "opus"), ("flac", "flac")] {
        let out = dir.path().join(format!("faixa2.{fmt}"));
        nago()
            .args(["convert", "--audio-track", "2", "-o"])
            .arg(&out)
            .arg(&input)
            .assert()
            .success();
        assert_eq!(stream_codecs(&out), [codec], "{fmt}");
    }
    // a faixa 2 é a de 880 Hz em inglês
    let probe = ffprobe_json(&dir.path().join("faixa2.flac"));
    assert_eq!(probe["streams"].as_array().unwrap().len(), 1);
    nago()
        .args(["convert", "--to", "m4a", "--audio-track", "3"])
        .arg(&input)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("2 faixa"));
}

/// `prepare_convert` (usado pela TUI para validar o formulário) dá o mesmo erro que a
/// CLI e não cria nenhum arquivo.
#[tokio::test(flavor = "multi_thread")]
async fn prepare_convert_da_o_mesmo_erro_da_cli_sem_criar_arquivos() {
    use nago_core::ops::{self, ConvertRequest, OutputOptions};
    use nago_core::output::Reservations;

    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("sem_audio.mp4");
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=25:duration=1",
            "-c:v",
            "libx264",
        ])
        .arg(&input)
        .status()
        .unwrap();
    assert!(ok.success());

    let tools = nago_core::tools::Tools::locate().await.unwrap();
    let caps = nago_core::caps::detect(&tools).await.unwrap();
    let req = ConvertRequest {
        input: input.clone(),
        output: OutputOptions::default(),
        format: Some("m4a".into()),
        audio_track: None,
        compat: false,
    };
    let err = ops::prepare_convert(&tools, &caps, &req, None, &Reservations::new())
        .await
        .unwrap_err();
    assert_eq!(err.exit_code(), 2);
    let names = || {
        let mut v: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        v.sort();
        v
    };
    assert_eq!(names(), ["sem_audio.mp4"]);

    let out = nago()
        .args(["convert", "--to", "m4a"])
        .arg(&input)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&err.to_string()),
        "CLI: {stderr}\ncore: {err}"
    );
    assert_eq!(names(), ["sem_audio.mp4"]);
}
