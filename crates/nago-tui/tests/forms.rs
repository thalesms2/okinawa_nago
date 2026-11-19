//! Spec `interactive-mode` › Formulários das operações.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossterm::event::KeyCode;
use nago_core::tools::Tools;
use nago_testkit::{Fixture, fixture_in};
use nago_tui::app::{Effect, Msg, Screen};
use nago_tui::effects::Env;
use nago_tui::form::FieldId;
use nago_tui::opform::{Check, OpRequest, Summary};
use support::{Harness, caps, entry};

/// Executa um `Effect::Prepare` de verdade e devolve a mensagem de resposta.
async fn run_prepare(env: &Env, effect: &Effect) -> Msg {
    let Effect::Prepare {
        seq,
        request,
        reserved,
    } = effect
    else {
        panic!("esperava Prepare, veio {effect:?}");
    };
    let result = env
        .prepare(request, reserved)
        .await
        .map(|p| Summary::from(&p))
        .map_err(|e| e.to_string());
    Msg::Prepared { seq: *seq, result }
}

fn prepare_effect(effects: Vec<Effect>) -> Effect {
    effects
        .into_iter()
        .find(|e| matches!(e, Effect::Prepare { .. }))
        .expect("um Prepare")
}

/// App com o arquivo listado e o formulário aberto pela tecla `key`.
fn open(dir: &Path, file: &str, key: char, working_hw: &[&str]) -> Harness {
    let (mut h, _) = Harness::with(dir, vec![], caps(working_hw), None);
    h.send(Msg::DirListed {
        dir: dir.to_path_buf(),
        result: Ok(vec![entry(dir, file, false)]),
    });
    h.char(key);
    assert_eq!(h.app.screen, Screen::Form);
    h
}

async fn env(working_hw: &[&str]) -> Arc<Env> {
    Arc::new(Env::new(Tools::locate().await.unwrap(), caps(working_hw)))
}

fn summary(h: &Harness) -> Summary {
    match &h.app.form.as_ref().unwrap().check {
        Check::Ready(s) => s.clone(),
        other => panic!("esperava resumo, veio {other:?}"),
    }
}

/// O caminho do diretório temporário varia; troca por `/tmp/teste` na tela.
fn stable_screen(h: &Harness, dir: &Path) -> String {
    let dir = dir.display().to_string();
    h.screen()
        .lines()
        .map(|l| {
            if l.contains(&dir) {
                // o comprimento do caminho varia: normaliza os espaços da linha
                let l = l.replace(&dir, "/tmp/teste");
                l.split_whitespace().collect::<Vec<_>>().join(" ") + "\n"
            } else {
                format!("{l}\n")
            }
        })
        .collect()
}

#[tokio::test]
async fn padroes_da_compressao() {
    let tmp = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, tmp.path());
    let env = env(&[]).await;
    let mut h = open(tmp.path(), "h264_aac_360p.mp4", 'z', &[]);
    // a validação inicial sai no primeiro tick
    let msg = run_prepare(&env, &prepare_effect(h.tick(0))).await;
    h.send(msg);
    let s = summary(&h);
    assert_eq!(s.encoder.as_deref(), Some("libsvtav1"));
    assert_eq!(s.quality.map(|q| q.name()), Some("high"));
    assert_eq!(s.output, tmp.path().join("h264_aac_360p.compressed.mp4"));
    assert!(
        !input.with_extension("compressed.mp4").exists(),
        "preparar não cria arquivos"
    );
    insta::assert_snapshot!("form_compressao", stable_screen(&h, tmp.path()));
}

#[tokio::test]
async fn gpu_indisponivel_mostra_o_encoder_de_software() {
    let tmp = tempfile::tempdir().unwrap();
    fixture_in(Fixture::H264Aac360p, tmp.path());
    let env = env(&[]).await;
    let mut h = open(tmp.path(), "h264_aac_360p.mp4", 'z', &[]);
    h.tick(0);
    // perfil → codec → usar GPU
    h.key(KeyCode::Down);
    h.key(KeyCode::Down);
    h.char(' ');
    assert!(h.app.form.as_ref().unwrap().form.toggle(FieldId::Gpu));
    // a validação é adiada enquanto o usuário edita
    assert!(h.tick(100).is_empty());
    let msg = run_prepare(&env, &prepare_effect(h.tick(300))).await;
    h.send(msg);
    let s = summary(&h);
    assert_eq!(s.encoder.as_deref(), Some("libsvtav1"));
    assert!(
        s.warnings
            .iter()
            .any(|w| w.contains("nenhum encoder de GPU funcional")),
        "{:?}",
        s.warnings
    );
    assert!(h.screen().contains("nenhum encoder de GPU funcional"));
}

#[tokio::test]
async fn gpu_disponivel_usa_nvenc() {
    let tmp = tempfile::tempdir().unwrap();
    fixture_in(Fixture::H264Aac360p, tmp.path());
    let working = ["av1_nvenc"];
    let env = env(&working).await;
    let mut h = open(tmp.path(), "h264_aac_360p.mp4", 'z', &working);
    h.key(KeyCode::Down);
    h.key(KeyCode::Down);
    h.char(' ');
    let msg = run_prepare(&env, &prepare_effect(h.tick(400))).await;
    h.send(msg);
    assert_eq!(summary(&h).encoder.as_deref(), Some("av1_nvenc"));
}

#[tokio::test]
async fn conversao_para_formato_sem_audio_mostra_o_erro_da_cli() {
    let tmp = tempfile::tempdir().unwrap();
    let input = tmp.path().join("sem_audio.mp4");
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
    let env = env(&[]).await;
    let mut h = open(tmp.path(), "sem_audio.mp4", 'v', &[]);
    // formato: mp4 → mkv → mov → webm → m4a
    for _ in 0..4 {
        h.key(KeyCode::Right);
    }
    assert_eq!(
        h.app.form.as_ref().unwrap().form.choice(FieldId::Format),
        "m4a"
    );
    let msg = run_prepare(&env, &prepare_effect(h.tick(400))).await;
    h.send(msg);
    let form = h.app.form.as_ref().unwrap();
    let Check::Invalid(msg) = &form.check else {
        panic!("esperava erro, veio {:?}", form.check);
    };
    // mesma mensagem que `nago convert --to m4a` (teste de paridade em nago-cli)
    assert!(msg.contains("áudio"), "{msg}");
    assert_eq!(
        form.form.first_error().map(|(id, _)| id),
        Some(FieldId::Format)
    );
    // Enter não enfileira com erro
    assert!(h.key(KeyCode::Enter).is_empty());
    assert_eq!(h.app.screen, Screen::Form);
    assert!(h.app.queue.jobs.is_empty());
    insta::assert_snapshot!("form_conversao_erro", stable_screen(&h, tmp.path()));
}

#[test]
fn erro_local_aponta_o_campo_e_bloqueia_o_envio() {
    let d = PathBuf::from("/v");
    let mut h = open(&d, "a.mp4", 'z', &[]);
    h.tick(0);
    // áudio (kbps) só aceita dígitos; um valor enorme não cabe em u32
    for _ in 0..4 {
        h.key(KeyCode::Down);
    }
    assert_eq!(
        h.app.form.as_ref().unwrap().form.focused().id,
        FieldId::AudioBitrate
    );
    h.type_str("99999999999");
    let form = h.app.form.as_ref().unwrap();
    assert_eq!(
        form.form.first_error().map(|(id, _)| id),
        Some(FieldId::AudioBitrate)
    );
    // nenhuma validação no core é agendada enquanto há erro local
    assert!(h.tick(1000).is_empty());
    assert!(h.key(KeyCode::Enter).is_empty());
    assert!(h.screen().contains("número inválido"));
    // corrigir o campo volta a validar
    for _ in 0..11 {
        h.key(KeyCode::Backspace);
    }
    h.type_str("96");
    assert!(matches!(&h.tick(400)[..], [Effect::Prepare { .. }]));
}

#[test]
fn esc_volta_ao_navegador_sem_enfileirar() {
    let d = PathBuf::from("/v");
    let mut h = open(&d, "a.mp4", 'v', &[]);
    h.key(KeyCode::Esc);
    assert_eq!(h.app.screen, Screen::Browser);
    assert!(h.app.form.is_none());
    assert!(h.tick(1000).is_empty(), "a validação pendente é descartada");
}

#[test]
fn texto_livre_aceita_q_e_interrogacao() {
    let d = PathBuf::from("/v");
    let mut h = open(&d, "a.mp4", 'v', &[]);
    // formato → faixa → compat → saída
    for _ in 0..3 {
        h.key(KeyCode::Down);
    }
    assert_eq!(
        h.app.form.as_ref().unwrap().form.focused().id,
        FieldId::Output
    );
    h.type_str("q?.mkv");
    assert_eq!(
        h.app.form.as_ref().unwrap().form.text(FieldId::Output),
        "q?.mkv"
    );
    assert!(h.app.overlay.is_none());
}

#[test]
fn selecao_multipla_gera_um_job_por_arquivo() {
    let d = PathBuf::from("/v");
    let (mut h, _) = Harness::with(&d, vec![], caps(&[]), None);
    h.send(Msg::DirListed {
        dir: d.clone(),
        result: Ok(vec![
            entry(&d, "a.mp4", false),
            entry(&d, "b.mp4", false),
            entry(&d, "c.mp4", false),
        ]),
    });
    for _ in 0..3 {
        h.char(' ');
    }
    h.char('z');
    let form = h.app.form.as_ref().unwrap();
    assert_eq!(form.files.len(), 3);
    assert!(h.screen().contains("(+2 arquivos)"));
    let effects = h.key(KeyCode::Enter);
    let [Effect::Enqueue { requests, .. }] = &effects[..] else {
        panic!("esperava Enqueue, veio {effects:?}");
    };
    let inputs: Vec<_> = requests.iter().map(|r| r.input().to_path_buf()).collect();
    assert_eq!(
        inputs,
        ["/v/a.mp4", "/v/b.mp4", "/v/c.mp4"].map(PathBuf::from)
    );
    // a resposta do core vira três jobs e o primeiro começa
    let results = requests
        .iter()
        .map(|r| {
            let out = r.input().with_extension("compressed.mp4");
            (r.clone(), Ok(out))
        })
        .collect();
    let effects = h.send(Msg::Enqueued { results });
    assert_eq!(h.app.queue.jobs.len(), 3);
    assert_eq!(h.app.screen, Screen::Browser);
    assert!(matches!(
        &effects[..],
        [Effect::StartJob {
            id: 1,
            request: OpRequest::Compress(_),
            ..
        }]
    ));
    assert!(h.screen().contains("3 jobs na fila"));
}
