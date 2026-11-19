//! `preview`: índice de keyframes e miniaturas (spec `frame-preview`).

use std::time::Duration;

use nago_core::preview::{self, png_dimensions};
use nago_core::probe;
use nago_core::tools::Tools;
use nago_testkit::{Fixture, fixture};

async fn tools() -> Tools {
    Tools::locate().await.expect("ffmpeg instalado")
}

#[tokio::test]
async fn indice_de_keyframes_com_gop_de_2s() {
    let t = tools().await;
    let info = probe::probe(&t, &fixture(Fixture::H264Aac1080p))
        .await
        .unwrap();
    let idx = preview::keyframe_index(&t, &info).await.unwrap().unwrap();
    assert_eq!(idx.len(), 5, "{:?}", idx.times());
    let near = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() < 0.01);
    assert!(near(idx.at_or_before(5.3), 4.0));
    assert!(near(idx.after(5.3), 6.0));
    assert!(near(idx.before(4.0), 2.0));
}

#[tokio::test]
async fn indice_concorda_com_a_consulta_da_cli() {
    let t = tools().await;
    for f in [Fixture::H264Aac1080p, Fixture::MkvMulti] {
        let info = probe::probe(&t, &fixture(f)).await.unwrap();
        let idx = preview::keyframe_index(&t, &info).await.unwrap().unwrap();
        for i in 0..20 {
            let at = info.duration_secs * f64::from(i) / 20.0;
            let cli = probe::keyframe_at_or_before(&t, &info, at)
                .await
                .unwrap()
                .unwrap();
            let local = idx.at_or_before(at).unwrap();
            assert!(
                (cli - local).abs() < 0.01,
                "{f:?} t={at}: cli {cli} índice {local}"
            );
        }
    }
}

#[tokio::test]
async fn sem_video_nao_ha_indice() {
    let t = tools().await;
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("musica.flac");
    let ok = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-f", "lavfi", "-i", "sine=duration=1"])
        .arg(&audio)
        .status()
        .unwrap();
    assert!(ok.success());
    let info = probe::probe(&t, &audio).await.unwrap();
    assert!(preview::keyframe_index(&t, &info).await.unwrap().is_none());
    assert!(
        preview::frame_png(&t, &info, 0.0, 320, 180, true)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn miniatura_cabe_na_caixa_mantendo_a_proporcao() {
    let t = tools().await;
    let info = probe::probe(&t, &fixture(Fixture::H264Aac1080p))
        .await
        .unwrap();
    let png = preview::frame_png(&t, &info, 3.5, 320, 320, true)
        .await
        .unwrap();
    let (w, h, depth) = png_dimensions(&png).expect("PNG válido");
    assert_eq!((w, h, depth), (320, 180, 8));
}

#[tokio::test]
async fn miniatura_hdr_sai_em_8_bits() {
    let t = tools().await;
    let info = probe::probe(&t, &fixture(Fixture::HevcHdr10))
        .await
        .unwrap();
    for tonemap in [true, false] {
        let png = preview::frame_png(&t, &info, 1.0, 320, 180, tonemap)
            .await
            .unwrap();
        let (w, h, depth) = png_dimensions(&png).expect("PNG válido");
        assert!(w <= 320 && h <= 180, "{w}x{h}");
        assert_eq!(depth, 8);
    }
}

/// Descartar o future (pedido obsoleto) encerra o ffmpeg.
#[cfg(unix)]
#[tokio::test]
async fn cancelar_a_extracao_mata_o_ffmpeg() {
    use std::os::unix::fs::PermissionsExt;

    let mut t = tools().await;
    let info = probe::probe(&t, &fixture(Fixture::H264Aac1080p))
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let fake = dir.path().join("ffmpeg-lento");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\necho $$ > {}\nexec sleep 30\n",
            pid_file.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    t.ffmpeg = fake;

    let task =
        tokio::spawn(async move { preview::frame_png(&t, &info, 1.0, 320, 180, true).await });
    let mut pid = None;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        if let Ok(s) = std::fs::read_to_string(&pid_file)
            && let Ok(p) = s.trim().parse::<i32>()
        {
            pid = Some(p);
            break;
        }
    }
    let pid = pid.expect("o ffmpeg falso começou");
    task.abort();
    let _ = task.await;
    let alive = |pid: i32| {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    for _ in 0..50 {
        if !alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("o processo {pid} continuou vivo após o cancelamento");
}
