use nago_core::probe::{self, HdrKind, StreamKind};
use nago_core::tools::Tools;
use nago_testkit::{Fixture, fixture};

async fn tools() -> Tools {
    Tools::locate().await.expect("ffmpeg instalado")
}

#[tokio::test]
async fn h264_aac_1080p() {
    let t = tools().await;
    let info = probe::probe(&t, &fixture(Fixture::H264Aac1080p))
        .await
        .unwrap();
    assert_eq!(info.container, "mp4");
    assert!(
        (info.duration_secs - 10.0).abs() < 0.1,
        "{}",
        info.duration_secs
    );
    assert!(info.size_bytes > 0);
    assert_eq!(info.streams.len(), 2);
    let v = info.primary_video().unwrap();
    assert_eq!(v.codec, "h264");
    let vi = v.video.as_ref().unwrap();
    assert_eq!((vi.width, vi.height), (1920, 1080));
    assert_eq!(vi.fps, Some(30.0));
    assert_eq!(vi.bit_depth, 8);
    assert_eq!(vi.hdr, None);
    let a = &info.streams[1];
    assert_eq!(a.kind, StreamKind::Audio);
    assert_eq!(a.codec, "aac");
    let ai = a.audio.as_ref().unwrap();
    assert_eq!(ai.channels, 2);
    assert_eq!(ai.sample_rate, Some(48000));
}

#[tokio::test]
async fn varias_faixas_com_idioma() {
    let t = tools().await;
    let info = probe::probe(&t, &fixture(Fixture::MkvMulti)).await.unwrap();
    assert_eq!(info.container, "matroska");
    let kinds: Vec<_> = info.streams.iter().map(|s| s.kind).collect();
    use StreamKind::*;
    assert_eq!(kinds, [Video, Audio, Audio, Subtitle, Subtitle]);
    let langs: Vec<_> = info.streams[1..]
        .iter()
        .map(|s| s.language.clone().unwrap())
        .collect();
    assert_eq!(langs, ["por", "eng", "por", "eng"]);
    assert_eq!(info.streams[3].codec, "subrip");
    assert!(info.streams[1].default);
}

#[tokio::test]
async fn hdr10_detectado_com_metadados() {
    let t = tools().await;
    let info = probe::probe(&t, &fixture(Fixture::HevcHdr10))
        .await
        .unwrap();
    let v = info.primary_video().unwrap().video.as_ref().unwrap();
    assert_eq!(v.hdr, Some(HdrKind::Hdr10));
    assert_eq!(v.bit_depth, 10);
    assert_eq!(v.color_primaries.as_deref(), Some("bt2020"));
    let md = v.mastering_display.unwrap();
    assert!((md.max_luminance - 1000.0).abs() < 1e-6);
    assert!((md.min_luminance - 0.005).abs() < 1e-9);
    assert!((md.green.0 - 0.265).abs() < 1e-9);
    let cl = v.content_light.unwrap();
    assert_eq!((cl.max_cll, cl.max_fall), (1000, 400));
    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["streams"][0]["video"]["hdr"], "HDR10");
}

#[tokio::test]
async fn sdr_tem_hdr_null_no_json() {
    let t = tools().await;
    let info = probe::probe(&t, &fixture(Fixture::H264Aac360p))
        .await
        .unwrap();
    let json = serde_json::to_value(&info).unwrap();
    assert!(json["streams"][0]["video"]["hdr"].is_null());
    assert!(
        json["streams"][0]["video"]
            .as_object()
            .unwrap()
            .contains_key("hdr")
    );
}

#[tokio::test]
async fn keyframe_anterior_com_gop_de_2s() {
    let t = tools().await;
    let info = probe::probe(&t, &fixture(Fixture::H264Aac1080p))
        .await
        .unwrap();
    let kf = probe::keyframe_at_or_before(&t, &info, 5.3)
        .await
        .unwrap()
        .unwrap();
    assert!((kf - 4.0).abs() < 0.01, "keyframe = {kf}");
    let kf = probe::keyframe_at_or_before(&t, &info, 4.0)
        .await
        .unwrap()
        .unwrap();
    assert!((kf - 4.0).abs() < 0.01, "keyframe = {kf}");
    let kf = probe::keyframe_at_or_before(&t, &info, 0.5)
        .await
        .unwrap()
        .unwrap();
    assert!(kf.abs() < 0.01, "keyframe = {kf}");
}

#[tokio::test]
async fn arquivo_inexistente_e_nao_midia() {
    let t = tools().await;
    let err = probe::probe(&t, std::path::Path::new("/nao/existe.mp4"))
        .await
        .unwrap_err();
    assert!(matches!(err, nago_core::Error::NotFound(_)));
    let dir = tempfile::tempdir().unwrap();
    let txt = dir.path().join("notas.txt");
    std::fs::write(&txt, "olá\nmundo\n").unwrap();
    let err = probe::probe(&t, &txt).await.unwrap_err();
    assert!(matches!(err, nago_core::Error::NotMedia(_)), "{err:?}");
}
