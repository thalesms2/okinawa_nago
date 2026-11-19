//! Modo interativo do `nago` (ratatui).
//!
//! Arquitetura (design D2 da change `add-tui`): o estado fica em [`app::App`], a
//! função pura [`app::update`] transforma mensagens em efeitos, [`effects`] executa os
//! efeitos (todo o I/O) e [`view`] desenha o estado.

pub mod app;
pub mod browser;
pub mod effects;
pub mod form;
pub mod keys;
pub mod layout;
pub mod opform;
pub mod queue;
mod term;
pub mod view;

use std::path::PathBuf;

use nago_core::caps;
use nago_core::tools::Tools;

/// Como abrir a TUI.
#[derive(Debug, Clone)]
pub struct TuiOptions {
    /// Pasta inicial do navegador.
    pub dir: PathBuf,
    /// Arquivos já selecionados (`nago tui a.mp4 b.mkv`).
    pub files: Vec<PathBuf>,
}

/// Resultado da sessão: o código de saída do processo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TuiExit {
    pub code: u8,
}

/// Abre a TUI e roda até o usuário sair.
///
/// Erros antes de abrir o terminal (ffmpeg ausente, arquivo inexistente) voltam como
/// erros do core, com seus códigos de saída.
pub async fn run(opts: TuiOptions) -> nago_core::Result<TuiExit> {
    for f in &opts.files {
        if !f.exists() {
            return Err(nago_core::Error::NotFound(f.clone()));
        }
    }
    let tools = Tools::locate().await?;
    let caps = caps::load_or_detect(&tools).await?;
    term::run(opts, tools, caps)
        .await
        .map_err(nago_core::Error::Io)
}
