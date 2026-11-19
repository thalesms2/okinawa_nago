//! Spec `video-cut`.

use std::path::Path;

use assert_cmd::Command;
use nago_testkit::{Fixture, duration_secs, ffprobe_json, fixture_in};
use predicates::prelude::*;

fn nago() -> Command {
    let mut c = Command::cargo_bin("nago").unwrap();
    c.env(
        "NAGO_CACHE_DIR",
        std::env::temp_dir().join("nago-test-cache"),
    );
    c
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// Duração do primeiro stream de vídeo (pts do último frame + 1 frame).
fn video_duration(path: &Path) -> f64 {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=nb_read_frames,avg_frame_rate",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let s = &v["streams"][0];
    let frames: f64 = s["nb_read_frames"].as_str().unwrap().parse().unwrap();
    let (n, d) = s["avg_frame_rate"]
        .as_str()
        .unwrap()
        .split_once('/')
        .unwrap();
    frames / (n.parse::<f64>().unwrap() / d.parse::<f64>().unwrap())
}

/// MD5 do frame decodificado `n` (0-based) de um arquivo, normalizado em yuv420p.
fn frame_md5(path: &Path, seek: Option<f64>, n: usize) -> String {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-v", "error"]);
    if let Some(s) = seek {
        cmd.args(["-ss", &format!("{s}")]);
    }
    let out = cmd
        .arg("-i")
        .arg(path)
        .args([
            "-map",
            "0:v:0",
            "-vf",
            &format!("select=eq(n\\,{n}),format=gray"),
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    format!("{:x}", md5_like(&out.stdout))
}

/// Hash simples (FNV-1a 64) — só para comparar frames, sem dependência extra.
fn md5_like(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ *b as u64).wrapping_mul(0x100000001b3)
    })
}

/// SSIM entre o primeiro frame de `a` e o frame no instante `t` de `b`.
fn ssim_first_frame(a: &Path, b: &Path, t: f64) -> f64 {
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "info", "-i"])
        .arg(a)
        .args(["-ss", &format!("{t}"), "-i"])
        .arg(b)
        .args(["-lavfi", "[0:v]trim=end_frame=1,setpts=PTS-STARTPTS[x];[1:v]trim=end_frame=1,setpts=PTS-STARTPTS[y];[x][y]ssim", "-f", "null", "-"])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    let all = err.split("All:").nth(1).expect("saída do ssim");
    all.split_whitespace().next().unwrap().parse().unwrap()
}

#[test]
fn intervalo_invalido_sai_com_2_sem_criar_arquivo() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    let cases: [(&[&str], &str); 5] = [
        (&["--from", "0:05", "--to", "0:03"], "anterior ao fim"),
        (&["--from", "1:00"], "além da duração"),
        (&["--from", "abc"], "instante inválido"),
        (&["--from", "-5"], "negativo"),
        (&["--from", "5", "--duration", "0"], "maior que zero"),
    ];
    for (args, msg) in cases {
        nago()
            .arg("cut")
            .arg(&input)
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains(msg));
    }
    // a duração real aparece na mensagem do início além do fim
    nago()
        .arg("cut")
        .arg(&input)
        .args(["--from", "1:00"])
        .assert()
        .stderr(predicate::str::contains("00:00:10.000"));
    assert_eq!(files_in(dir.path()), ["h264_aac_360p.mp4"]);
}

#[test]
fn to_e_duration_juntos_sao_rejeitados() {
    nago()
        .args(["cut", "a.mp4", "--to", "5", "--duration", "2"])
        .assert()
        .code(2);
}

#[test]
fn fim_alem_da_duracao_vai_ate_o_fim_com_aviso() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    nago()
        .arg("cut")
        .arg(&input)
        .args(["--from", "8", "--to", "10:00"])
        .assert()
        .success()
        .stderr(
            predicate::str::contains("aviso").and(predicate::str::contains("passa da duração")),
        );
    let d = duration_secs(&dir.path().join("h264_aac_360p.cut.mp4"));
    assert!((d - 2.0).abs() < 0.15, "duração {d}");
}

#[test]
fn corte_rapido_comeca_no_keyframe_e_avisa() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac1080p, dir.path());
    let out = nago()
        .args(["cut", "--json", "--from", "5.3", "--to", "8"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("00:00:04.000") && stderr.contains("--exact"),
        "{stderr}"
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!((report["actual_start_secs"].as_f64().unwrap() - 4.0).abs() < 0.01);
    assert_eq!(report["requested_start_secs"], 5.3);
    assert!(report["encoder"].is_null(), "nada é recodificado");
    let output = dir.path().join("h264_aac_1080p.cut.mp4");
    assert!(
        (video_duration(&output) - 4.0).abs() < 0.05,
        "vídeo de 4,0 s a 8,0 s"
    );
    let probe = ffprobe_json(&output);
    let audio_dur: f64 = probe["streams"][1]["duration"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((audio_dur - 4.0).abs() < 0.1, "áudio {audio_dur}");
    // o primeiro frame da saída é o keyframe de 4,0 s do original (sem recodificar)
    assert_eq!(frame_md5(&output, None, 0), frame_md5(&input, None, 120));
}

#[test]
fn corte_preserva_todas_as_faixas_e_idiomas() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::MkvMulti, dir.path());
    nago()
        .arg("cut")
        .arg(&input)
        .args(["--from", "2", "--to", "5"])
        .assert()
        .success();
    let probe = ffprobe_json(&dir.path().join("multi_tracks.cut.mkv"));
    let streams: Vec<(String, String)> = probe["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["codec_type"].as_str().unwrap().to_owned(),
                s["tags"]["language"].as_str().unwrap_or("-").to_owned(),
            )
        })
        .collect();
    let expected = [
        ("video", "-"),
        ("audio", "por"),
        ("audio", "eng"),
        ("subtitle", "por"),
        ("subtitle", "eng"),
    ]
    .map(|(a, b)| (a.to_owned(), b.to_owned()));
    assert_eq!(streams, expected);
}

#[test]
fn nome_padrao_e_colisao() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    for _ in 0..2 {
        nago()
            .arg("cut")
            .arg(&input)
            .args(["--from", "1", "--duration", "2"])
            .assert()
            .success();
    }
    assert_eq!(
        files_in(dir.path()),
        [
            "h264_aac_360p.cut-1.mp4",
            "h264_aac_360p.cut.mp4",
            "h264_aac_360p.mp4"
        ]
    );
}

#[test]
fn corte_exato_tem_precisao_de_um_frame() {
    let dir = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, dir.path());
    let out = nago()
        .args(["cut", "--exact", "--json", "--from", "1.5", "--to", "7.5"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["encoder"], "libx264", "mesmo codec da entrada");
    assert_eq!(report["quality"], "visually-lossless");
    let output = dir.path().join("h264_aac_360p.cut.mp4");
    let d = video_duration(&output);
    assert!((d - 6.0).abs() <= 1.0 / 30.0 + 1e-6, "duração do vídeo {d}");
    let ssim = ssim_first_frame(&output, &input, 1.5);
    assert!(
        ssim >= 0.99,
        "primeiro frame difere do original em 1,5 s (SSIM {ssim})"
    );
    // áudio acompanha o vídeo
    let probe = ffprobe_json(&output);
    let audio_dur: f64 = probe["streams"][1]["duration"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((audio_dur - 6.0).abs() < 0.1, "áudio {audio_dur}");
}

/// Texto humano do relatório (caminho e tempo variam).
#[test]
fn relatorio_texto_humano_do_corte() {
    let dir = tempfile::tempdir().unwrap();
    let input = nago_testkit::fixture_in(nago_testkit::Fixture::MkvMulti, dir.path());
    let out = assert_cmd::Command::cargo_bin("nago")
        .unwrap()
        .args(["cut", "--from", "1", "--to", "3"])
        .arg(&input)
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    let body: Vec<String> = text
        .lines()
        .skip(1)
        .map(|l| match l.rsplit_once(" em ") {
            Some((head, _)) if l.contains('%') => format!("{head} em [tempo]"),
            _ => l.to_owned(),
        })
        .collect();
    insta::assert_snapshot!(body.join("\n"));
}
