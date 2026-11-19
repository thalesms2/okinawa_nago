//! Spec `job-queue` › Saída com fila ativa e Códigos de saída da TUI.

mod support;

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyModifiers};
use nago_core::ops::{CutRequest, OutputOptions};
use nago_tui::app::{Effect, Msg, Overlay};
use nago_tui::opform::OpRequest;
use nago_tui::queue::{FailKind, JobFailure};
use support::{Harness, caps};

fn harness() -> Harness {
    Harness::with(Path::new("/v"), vec![], caps(&[]), None).0
}

fn cut(name: &str) -> OpRequest {
    OpRequest::Cut(CutRequest {
        input: PathBuf::from(format!("/v/{name}.mp4")),
        output: OutputOptions::default(),
        from: None,
        to: Some(1.0),
        duration: None,
        exact: false,
    })
}

/// Um job em execução e `pending` pendentes.
fn busy(pending: usize) -> Harness {
    let mut h = harness();
    for i in 0..=pending {
        let name = format!("v{i}");
        h.send(Msg::Enqueued {
            results: vec![(cut(&name), Ok(PathBuf::from(format!("/v/{name}.cut.mp4"))))],
        });
    }
    h.send(Msg::JobStarted {
        id: 1,
        output: "/v/v0.cut.mp4".into(),
    });
    h
}

fn finished(kind: FailKind) -> JobFailure {
    JobFailure {
        kind,
        message: "x".into(),
        log_tail: vec![],
    }
}

fn ctrl_c(h: &mut Harness) -> Vec<Effect> {
    h.key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL)
}

#[test]
fn sem_jobs_sai_direto_com_0() {
    let mut h = harness();
    assert!(matches!(&h.char('q')[..], [Effect::Exit(0)]));
    let mut h = harness();
    assert!(matches!(&ctrl_c(&mut h)[..], [Effect::Exit(0)]));
}

#[test]
fn jobs_terminados_com_falha_nao_mudam_o_codigo() {
    let mut h = busy(1);
    h.send(Msg::JobFinished {
        id: 1,
        result: Err(finished(FailKind::Failed)),
    });
    h.send(Msg::JobFinished {
        id: 2,
        result: Err(finished(FailKind::Discarded)),
    });
    assert!(matches!(&h.char('q')[..], [Effect::Exit(0)]));
}

#[test]
fn sair_durante_a_compressao_pede_confirmacao() {
    let mut h = busy(2);
    assert!(h.char('q').is_empty());
    assert_eq!(h.app.overlay, Some(Overlay::ConfirmQuit));
    let screen = h.screen();
    assert!(
        screen.contains("o job em execução será cancelado"),
        "{screen}"
    );
    assert!(screen.contains("2 job(s) pendente(s) serão descartados"));
    insta::assert_snapshot!("confirmar_saida", screen);
    // desistir
    assert!(h.char('n').is_empty());
    assert_eq!(h.app.overlay, None);
    assert_eq!(h.app.queue.active_count(), 3);
    // confirmar: cancela o atual, descarta os pendentes e espera o cancelamento
    h.char('q');
    let e = h.char('s');
    assert!(matches!(&e[..], [Effect::CancelJob(1)]));
    assert_eq!(h.app.queue.pending_count(), 0);
    assert_eq!(h.app.quitting, Some(130));
    let e = h.send(Msg::JobFinished {
        id: 1,
        result: Err(finished(FailKind::Cancelled)),
    });
    assert!(matches!(&e[..], [Effect::Exit(130)]), "{e:?}");
}

#[test]
fn ctrl_c_duas_vezes_confirma() {
    let mut h = busy(0);
    assert!(ctrl_c(&mut h).is_empty());
    assert_eq!(h.app.overlay, Some(Overlay::ConfirmQuit));
    let e = ctrl_c(&mut h);
    assert!(matches!(&e[..], [Effect::CancelJob(1)]));
    assert_eq!(h.app.quitting, Some(130));
}

#[test]
fn ctrl_c_espacado_so_pergunta() {
    let mut h = busy(0);
    ctrl_c(&mut h);
    h.char('n');
    h.clock.advance(2500);
    assert!(ctrl_c(&mut h).is_empty());
    assert_eq!(h.app.overlay, Some(Overlay::ConfirmQuit));
    h.clock.advance(2500);
    assert!(
        ctrl_c(&mut h).is_empty(),
        "fora da janela de 2 s não confirma"
    );
}

#[test]
fn sigterm_com_job_cancela_e_sai_com_130() {
    let mut h = busy(1);
    let e = h.send(Msg::Terminate);
    assert!(matches!(&e[..], [Effect::CancelJob(1)]));
    let e = h.send(Msg::JobFinished {
        id: 1,
        result: Err(finished(FailKind::Cancelled)),
    });
    assert!(matches!(&e[..], [Effect::Exit(130)]));
}

#[test]
fn sigterm_sem_jobs_sai_com_0() {
    let mut h = harness();
    assert!(matches!(&h.send(Msg::Terminate)[..], [Effect::Exit(0)]));
}

#[test]
fn nenhum_job_comeca_depois_da_confirmacao() {
    let mut h = busy(0);
    h.char('q');
    h.char('s');
    // um envio que chegue atrasado não inicia nada
    let e = h.send(Msg::Enqueued {
        results: vec![(cut("tarde"), Ok("/v/tarde.cut.mp4".into()))],
    });
    assert!(!e.iter().any(|e| matches!(e, Effect::StartJob { .. })));
}
