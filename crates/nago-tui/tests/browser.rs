//! Spec `interactive-mode`: navegador, painel de metadados e ajuda.

mod support;

use std::path::{Path, PathBuf};

use crossterm::event::KeyCode;
use nago_core::probe;
use nago_core::tools::Tools;
use nago_testkit::{Fixture, fixture};
use nago_tui::app::{Effect, InfoState, Msg, Screen};
use support::{Harness, entry, probes};

fn dir() -> PathBuf {
    PathBuf::from("/v")
}

fn names(h: &Harness) -> Vec<String> {
    h.app
        .browser
        .visible()
        .into_iter()
        .map(|e| e.name)
        .collect()
}

#[test]
fn filtro_de_extensoes() {
    let d = dir();
    let (mut h, _) = Harness::listed(
        &d,
        vec![
            entry(&d, "musica.flac", false),
            entry(&d, "notas.txt", false),
            entry(&d, "video.mp4", false),
        ],
    );
    assert_eq!(names(&h), ["..", "musica.flac", "video.mp4"]);
    insta::assert_snapshot!("navegador_so_midia", h.screen());
    h.char('.');
    assert_eq!(names(&h), ["..", "musica.flac", "notas.txt", "video.mp4"]);
    assert!(h.screen().contains("notas.txt"));
    assert!(h.screen().contains("arquivos (todos)"));
}

#[test]
fn navegacao_entre_pastas() {
    let d = dir();
    let (mut h, _) = Harness::listed(
        &d,
        vec![entry(&d, "fotos", true), entry(&d, "a.mp4", false)],
    );
    // o cursor começa no primeiro item depois de `..`
    assert_eq!(h.app.browser.selected().unwrap().name, "fotos");
    let effects = h.key(KeyCode::Enter);
    assert!(matches!(&effects[..], [Effect::ListDir(p)] if p == Path::new("/v/fotos")));
    assert!(h.app.browser.loading);
    h.send(Msg::DirListed {
        dir: "/v/fotos".into(),
        result: Ok(vec![]),
    });
    // voltar seleciona a pasta de onde viemos
    let effects = h.key(KeyCode::Backspace);
    assert!(matches!(&effects[..], [Effect::ListDir(p)] if p == Path::new("/v")));
    h.send(Msg::DirListed {
        dir: d.clone(),
        result: Ok(vec![entry(&d, "fotos", true), entry(&d, "a.mp4", false)]),
    });
    assert_eq!(h.app.browser.selected().unwrap().name, "fotos");
    // uma listagem atrasada de outra pasta é ignorada
    h.send(Msg::DirListed {
        dir: "/outra".into(),
        result: Ok(vec![]),
    });
    assert_eq!(names(&h), ["..", "fotos", "a.mp4"]);
}

#[test]
fn erro_ao_ler_pasta_aparece_na_lista() {
    let d = dir();
    let (mut h, _) = Harness::with(&d, vec![], support::caps(&[]), None);
    h.send(Msg::DirListed {
        dir: d,
        result: Err("não foi possível ler a pasta: permissão negada".into()),
    });
    assert!(h.screen().contains("permissão negada"));
}

#[test]
fn marcacao_multipla() {
    let d = dir();
    let (mut h, _) = Harness::listed(
        &d,
        vec![
            entry(&d, "a.mp4", false),
            entry(&d, "b.mp4", false),
            entry(&d, "c.mp4", false),
        ],
    );
    h.char(' ');
    h.char(' ');
    assert_eq!(h.app.browser.marked.len(), 2);
    assert!(h.screen().contains("[x] a.mp4"));
    assert!(h.screen().contains("[ ] c.mp4"));
}

#[test]
fn rolar_rapido_dispara_um_so_probe() {
    let d = dir();
    let files: Vec<_> = (0..6)
        .map(|i| entry(&d, &format!("v{i}.mp4"), false))
        .collect();
    let (mut h, effects) = Harness::listed(&d, files);
    // a listagem lê o arquivo destacado na hora
    assert_eq!(probes(&effects), [PathBuf::from("/v/v0.mp4")]);
    let mut all = Vec::new();
    for _ in 0..4 {
        all.extend(h.key(KeyCode::Down));
    }
    all.extend(h.tick(100));
    assert!(probes(&all).is_empty(), "nenhum probe antes do atraso");
    let fired = h.tick(100);
    assert_eq!(probes(&fired), [PathBuf::from("/v/v4.mp4")]);
    assert!(probes(&h.tick(500)).is_empty());
    assert!(h.screen().contains("lendo…"));
}

#[test]
fn arquivo_ja_lido_nao_dispara_probe() {
    let d = dir();
    let (mut h, _) = Harness::listed(
        &d,
        vec![entry(&d, "a.mp4", false), entry(&d, "b.mp4", false)],
    );
    h.send(Msg::Probed {
        path: "/v/b.mp4".into(),
        result: Err("x".into()),
    });
    h.key(KeyCode::Down);
    assert!(probes(&h.tick(500)).is_empty());
}

#[tokio::test]
async fn painel_de_metadados_da_fixture() {
    let tools = Tools::locate().await.unwrap();
    let path = fixture(Fixture::H264Aac360p);
    let info = probe::probe(&tools, &path).await.unwrap();
    let d = path.parent().unwrap().to_path_buf();
    let (mut h, _) = Harness::listed(&d, vec![entry(&d, "h264_aac_360p.mp4", false)]);
    h.send(Msg::Probed {
        path: path.clone(),
        result: Ok(info),
    });
    assert!(matches!(h.app.info(&path), Some(InfoState::Ready(_))));
    // o cabeçalho mostra a pasta, que varia entre máquinas
    let screen: String = h
        .screen()
        .lines()
        .skip(1)
        .map(|l| format!("{l}\n"))
        .collect();
    insta::assert_snapshot!("painel_metadados_h264", screen);
}

#[tokio::test]
async fn arquivo_corrompido_mostra_o_motivo_e_a_navegacao_continua() {
    let tools = Tools::locate().await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("corrompido.mp4");
    std::fs::write(&real, b"isto nao e um video").unwrap();
    let err = probe::probe(&tools, &real).await.unwrap_err().to_string();
    let d = dir();
    let shown = d.join("corrompido.mp4");
    let message = err.replace(&real.display().to_string(), &shown.display().to_string());
    let (mut h, _) = Harness::listed(
        &d,
        vec![
            entry(&d, "corrompido.mp4", false),
            entry(&d, "ok.mp4", false),
        ],
    );
    h.send(Msg::Probed {
        path: shown,
        result: Err(message),
    });
    insta::assert_snapshot!("painel_arquivo_corrompido", h.screen());
    h.key(KeyCode::Down);
    assert_eq!(h.app.browser.selected().unwrap().name, "ok.mp4");
    assert_eq!(h.app.screen, Screen::Browser);
}

#[test]
fn ajuda_no_navegador() {
    let d = dir();
    let (mut h, _) = Harness::listed(&d, vec![entry(&d, "a.mp4", false)]);
    h.char('?');
    insta::assert_snapshot!("ajuda_navegador", h.screen());
    h.key(KeyCode::Esc);
    assert!(!h.screen().contains("atalhos"));
}

#[test]
fn ajuda_no_corte_e_na_fila() {
    let d = dir();
    let (mut h, _) = Harness::listed(&d, vec![entry(&d, "a.mp4", false)]);
    h.char('c');
    assert_eq!(h.app.screen, Screen::Form);
    h.char('?');
    insta::assert_snapshot!("ajuda_corte", h.screen());
    h.char('?');
    h.key(KeyCode::Esc);
    h.key(KeyCode::Tab);
    assert_eq!(h.app.screen, Screen::Queue);
    h.char('?');
    insta::assert_snapshot!("ajuda_fila", h.screen());
}
