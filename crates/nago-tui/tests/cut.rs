//! Spec `frame-preview`: escolha do trecho, keyframes e miniatura.

mod support;

use std::path::Path;

use crossterm::event::{KeyCode, KeyModifiers};
use image::DynamicImage;
use nago_core::preview::KeyframeIndex;
use nago_core::probe::{self, MediaInfo};
use nago_core::tools::Tools;
use nago_testkit::{Fixture, fixture};
use nago_tui::app::{Effect, Msg, ThumbSupport};
use nago_tui::effects::Env;
use nago_tui::form::FieldId;
use nago_tui::opform::{Check, Keyframes, Summary};
use support::{Harness, caps, entry};

async fn info_of(path: &Path) -> MediaInfo {
    let tools = Tools::locate().await.unwrap();
    probe::probe(&tools, path).await.unwrap()
}

/// Formulário de corte aberto com o probe já feito.
fn open_cut(info: MediaInfo, thumbs: Option<ThumbSupport>) -> (Harness, Vec<Effect>) {
    let path = info.path.clone();
    let dir = path.parent().unwrap().to_path_buf();
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let (mut h, _) = Harness::with(&dir, vec![], caps(&[]), thumbs);
    h.send(Msg::DirListed {
        dir: dir.clone(),
        result: Ok(vec![entry(&dir, &name, false)]),
    });
    h.send(Msg::Probed {
        path,
        result: Ok(info),
    });
    let effects = h.char('c');
    (h, effects)
}

fn text(h: &Harness, id: FieldId) -> String {
    h.app.form.as_ref().unwrap().form.text(id).to_owned()
}

fn set(h: &mut Harness, id: FieldId, value: &str) {
    let form = h.app.form.as_mut().unwrap();
    form.form.focus_on(id);
    form.form.set_text(id, value.to_owned());
}

fn index(times: &[f64]) -> KeyframeIndex {
    KeyframeIndex::from_times(times.to_vec())
}

#[tokio::test]
async fn abre_com_o_trecho_inteiro_e_indexa_keyframes() {
    let info = info_of(&fixture(Fixture::H264Aac360p)).await;
    let path = info.path.clone();
    let (h, effects) = open_cut(info, None);
    assert_eq!(text(&h, FieldId::Start), "00:00:00.000");
    assert_eq!(text(&h, FieldId::End), "00:00:10.000");
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::IndexKeyframes(p) if *p == path))
    );
    assert!(h.screen().contains("indexando keyframes…"));
}

#[tokio::test]
async fn passo_de_um_frame() {
    let info = info_of(&fixture(Fixture::H264Aac360p)).await;
    let (mut h, _) = open_cut(info, None);
    set(&mut h, FieldId::Start, "00:00:05.000");
    h.char('>');
    assert_eq!(text(&h, FieldId::Start), "00:00:05.033");
    h.char('<');
    h.char('<');
    assert_eq!(text(&h, FieldId::Start), "00:00:04.967");
    // `o` passa a editar o fim; passos de 1 s e 10 s
    h.char('o');
    set(&mut h, FieldId::End, "00:00:08.000");
    h.key(KeyCode::Left);
    assert_eq!(text(&h, FieldId::End), "00:00:07.000");
    h.key_mod(KeyCode::Right, KeyModifiers::SHIFT);
    assert_eq!(text(&h, FieldId::End), "00:00:10.000", "limitado à duração");
    h.char('i');
    h.key_mod(KeyCode::Left, KeyModifiers::SHIFT);
    assert_eq!(
        text(&h, FieldId::Start),
        "00:00:00.000",
        "não fica negativo"
    );
}

#[tokio::test]
async fn intervalo_invalido_bloqueia_o_envio() {
    let info = info_of(&fixture(Fixture::H264Aac360p)).await;
    let env = Env::new(Tools::locate().await.unwrap(), caps(&[]));
    let (mut h, _) = open_cut(info, None);
    set(&mut h, FieldId::End, "00:00:03.000");
    h.char('i');
    set(&mut h, FieldId::Start, "00:00:04.000");
    h.key(KeyCode::Right);
    let effects = h.tick(400);
    let Some(Effect::Prepare {
        seq,
        request,
        reserved,
    }) = effects.iter().find(|e| matches!(e, Effect::Prepare { .. }))
    else {
        panic!("esperava Prepare: {effects:?}");
    };
    let result = env
        .prepare(request, reserved)
        .await
        .map(|p| Summary::from(&p))
        .map_err(|e| e.to_string());
    h.send(Msg::Prepared { seq: *seq, result });
    let form = h.app.form.as_ref().unwrap();
    let Check::Invalid(msg) = &form.check else {
        panic!("esperava erro: {:?}", form.check);
    };
    assert!(msg.contains("deve ser anterior ao fim"), "{msg}");
    assert_eq!(
        form.form.first_error().map(|(id, _)| id),
        Some(FieldId::Start)
    );
    assert!(h.key(KeyCode::Enter).is_empty());
    assert!(h.app.queue.jobs.is_empty());
}

#[tokio::test]
async fn corte_de_audio_sem_miniatura_nem_keyframes() {
    let tmp = tempfile::tempdir().unwrap();
    let flac = tmp.path().join("musica.flac");
    let ok = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-f", "lavfi", "-i", "sine=duration=20"])
        .arg(&flac)
        .status()
        .unwrap();
    assert!(ok.success());
    let info = info_of(&flac).await;
    let thumbs = ThumbSupport {
        picker: None,
        font: (10, 20),
    };
    let (mut h, effects) = open_cut(info, Some(thumbs));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::IndexKeyframes(_)))
    );
    let cut = h.app.form.as_ref().unwrap().cut.as_ref().unwrap();
    assert_eq!(cut.keyframes, Keyframes::NoVideo);
    // passos de tempo funcionam; saltos de keyframe não fazem nada
    h.key(KeyCode::Right);
    assert_eq!(text(&h, FieldId::Start), "00:00:01.000");
    h.char(']');
    assert_eq!(text(&h, FieldId::Start), "00:00:01.000");
    let all: Vec<Effect> = [h.tick(200), h.tick(500)].into_iter().flatten().collect();
    assert!(!all.iter().any(|e| matches!(e, Effect::Frame { .. })));
    let screen = h.screen();
    assert!(screen.contains("arquivo sem vídeo"));
    assert!(!screen.contains("frame do"));
}

#[tokio::test]
async fn ajuste_por_keyframe() {
    let info = info_of(&fixture(Fixture::H264Aac360p)).await;
    let path = info.path.clone();
    let (mut h, _) = open_cut(info, None);
    // antes do índice, o salto avisa que está indexando
    h.char(']');
    assert!(h.screen().contains("indexando keyframes"));
    h.send(Msg::KeyframesIndexed {
        path,
        result: Ok(Some(index(&[0.0, 2.0, 4.0, 6.0, 8.0]))),
    });
    set(&mut h, FieldId::Start, "00:00:03.500");
    h.char(']');
    assert_eq!(text(&h, FieldId::Start), "00:00:04.000");
    h.char(']');
    assert_eq!(text(&h, FieldId::Start), "00:00:06.000");
    h.char('[');
    h.char('[');
    assert_eq!(text(&h, FieldId::Start), "00:00:02.000");
}

#[tokio::test]
async fn keyframe_distante_sugere_o_modo_exato() {
    let tmp = tempfile::tempdir().unwrap();
    let long = tmp.path().join("longo.mp4");
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x90:rate=10:duration=60",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
        ])
        .arg(&long)
        .status()
        .unwrap();
    assert!(ok.success());
    let info = info_of(&long).await;
    let path = info.path.clone();
    let (mut h, _) = open_cut(info, None);
    h.send(Msg::KeyframesIndexed {
        path,
        result: Ok(Some(index(&[0.0, 10.0, 38.5, 50.0]))),
    });
    set(&mut h, FieldId::Start, "00:00:42.000");
    let form = h.app.form.as_ref().unwrap();
    let (kf, diff) = form.fast_cut_start().unwrap();
    assert_eq!(kf, 38.5);
    assert!((diff - 3.5).abs() < 1e-9);
    let screen = h.screen();
    assert!(screen.contains("00:00:38.500 (3.5 s antes)"), "{screen}");
    assert!(screen.contains("considere o modo exato"));
    let screen = screen.replace(&tmp.path().display().to_string(), "/tmp/teste");
    insta::assert_snapshot!(
        "corte_keyframe_distante",
        screen
            .lines()
            .skip(1)
            .map(|l| format!("{l}\n"))
            .collect::<String>()
    );
    // no modo exato o aviso some
    let form = h.app.form.as_mut().unwrap();
    form.form.focus_on(FieldId::Exact);
    h.char(' ');
    assert!(!h.screen().contains("considere o modo exato"));
    assert!(h.screen().contains("modo exato: recodifica"));
}

#[tokio::test]
async fn ajustes_seguidos_exibem_so_o_ultimo_frame() {
    let info = info_of(&fixture(Fixture::H264Aac360p)).await;
    let thumbs = ThumbSupport {
        picker: None,
        font: (10, 20),
    };
    let (mut h, _) = open_cut(info, Some(thumbs));
    // primeiro frame (início = 0) ao abrir
    let first = h.tick(0);
    let [
        Effect::Frame {
            seq: s1, key: k1, ..
        },
    ] = first
        .iter()
        .filter(|e| matches!(e, Effect::Frame { .. }))
        .collect::<Vec<_>>()[..]
    else {
        panic!("esperava um Frame: {first:?}");
    };
    let (s1, k1) = (*s1, *k1);
    assert_eq!(k1.0, 0);
    // 30 frames à frente, uma tecla a cada 10 ms
    let mut during = Vec::new();
    for _ in 0..30 {
        during.extend(h.char('>'));
    }
    assert!(during.iter().all(|e| !matches!(e, Effect::Frame { .. })));
    let after = h.tick(120);
    let frames: Vec<_> = after
        .iter()
        .filter_map(|e| match e {
            Effect::Frame { seq, key, t, .. } => Some((*seq, *key, *t)),
            _ => None,
        })
        .collect();
    assert_eq!(frames.len(), 1, "um único pedido depois da sequência");
    let (s2, k2, t2) = frames[0];
    assert!(
        (t2 - 1.0).abs() < 1e-9,
        "30 frames a 30 fps = 1 s, veio {t2}"
    );
    // a resposta antiga chega depois e é descartada
    let img = || DynamicImage::new_rgb8(4, 4);
    h.send(Msg::Frame {
        seq: s2,
        key: k2,
        result: Ok(img()),
    });
    h.send(Msg::Frame {
        seq: s1,
        key: k1,
        result: Ok(img()),
    });
    let cut = h.app.form.as_ref().unwrap().cut.as_ref().unwrap();
    assert_eq!(cut.thumb.shown.as_ref().unwrap().key, k2);
    assert!(!cut.thumb.loading);
    // voltar ao instante anterior usa o cache, sem novo pedido
    for _ in 0..30 {
        h.char('<');
    }
    let back = h.tick(120);
    assert!(!back.iter().any(|e| matches!(e, Effect::Frame { .. })));
    let cut = h.app.form.as_ref().unwrap().cut.as_ref().unwrap();
    assert_eq!(cut.thumb.shown.as_ref().unwrap().key, k1);
}

#[tokio::test]
async fn sem_protocolo_de_imagem_nenhum_frame_e_pedido() {
    let info = info_of(&fixture(Fixture::H264Aac360p)).await;
    let (mut h, _) = open_cut(info, None);
    let mut all = h.tick(0);
    for _ in 0..5 {
        all.extend(h.key(KeyCode::Right));
    }
    all.extend(h.tick(500));
    assert!(!all.iter().any(|e| matches!(e, Effect::Frame { .. })));
    let screen = h.screen();
    assert!(!screen.contains("frame do"));
    assert!(screen.contains("trecho:"));
}

/// Com um `Picker` kitty, o frame recebido vira um protocolo de imagem desenhado na tela.
#[tokio::test]
async fn frame_vira_imagem_kitty_na_tela() {
    use ratatui_image::picker::{Picker, ProtocolType};

    let info = info_of(&fixture(Fixture::H264Aac360p)).await;
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Kitty);
    let font = picker.font_size();
    let thumbs = ThumbSupport {
        picker: Some(picker),
        font: (font.width, font.height),
    };
    let (mut h, _) = open_cut(info, Some(thumbs));
    let effects = h.tick(0);
    let Some(&Effect::Frame { seq, key, .. }) =
        effects.iter().find(|e| matches!(e, Effect::Frame { .. }))
    else {
        panic!("esperava Frame: {effects:?}");
    };
    h.send(Msg::Frame {
        seq,
        key,
        result: Ok(DynamicImage::new_rgb8(key.1.min(320), key.2.min(180))),
    });
    let cut = h.app.form.as_ref().unwrap().cut.as_ref().unwrap();
    assert!(cut.thumb.shown.as_ref().unwrap().protocol.is_some());
    // a sequência de imagem do kitty começa com ESC _ G
    let screen = support::render(&h.app, 120, 30);
    assert!(screen.contains("\u{1b}_G"), "sem sequência kitty na tela");
    assert!(screen.contains("frame do início · 00:00:00.000"));
}
