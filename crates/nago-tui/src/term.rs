//! Terminal e laço principal.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use nago_core::caps::Capabilities;
use nago_core::tools::Tools;
use ratatui_image::picker::{Picker, ProtocolType};
use tokio::sync::mpsc::unbounded_channel;

use crate::app::{App, AppInit, Effect, Msg, ThumbSupport, update};
use crate::effects::{EffectRunner, Env};
use crate::view;
use crate::{TuiExit, TuiOptions};

/// Restaura o terminal ao sair do escopo, inclusive em erro.
struct RestoreGuard;

impl Drop for RestoreGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

/// Tempo máximo esperando o job cancelado terminar ao sair.
const QUIT_GRACE: Duration = Duration::from_secs(5);

/// Detecta o protocolo de imagem do terminal (`NAGO_TUI_IMAGES` força ou desliga).
/// Sem protocolo gráfico (só blocos de texto), a miniatura fica desativada.
fn detect_thumbs() -> Option<ThumbSupport> {
    let mode = std::env::var("NAGO_TUI_IMAGES")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if mode == "off" {
        return None;
    }
    let mut picker = match Picker::from_query_stdio() {
        Ok(p) => p,
        Err(_) => {
            release_query_thread();
            return None;
        }
    };
    match mode.as_str() {
        "kitty" => picker.set_protocol_type(ProtocolType::Kitty),
        "sixel" => picker.set_protocol_type(ProtocolType::Sixel),
        "iterm2" => picker.set_protocol_type(ProtocolType::Iterm2),
        _ => {}
    }
    if picker.protocol_type() == ProtocolType::Halfblocks {
        return None;
    }
    let font = picker.font_size();
    Some(ThumbSupport {
        font: (font.width.max(1), font.height.max(1)),
        picker: Some(picker),
    })
}

/// Quando a consulta do terminal falha (sem resposta dentro do prazo), a thread de
/// consulta do `ratatui-image` continua bloqueada lendo o stdin e roubaria as teclas.
/// Um pedido de status (`ESC [ 5 n`), que praticamente todo terminal responde, entrega
/// a ela a resposta que espera para terminar. Roda antes de o crossterm ler o teclado.
fn release_query_thread() {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = out.write_all(b"\x1b[5n");
    let _ = out.flush();
    std::thread::sleep(Duration::from_millis(150));
}

pub async fn run(opts: TuiOptions, tools: Tools, caps: Capabilities) -> std::io::Result<TuiExit> {
    let mut terminal = ratatui::try_init()?;
    let _guard = RestoreGuard;
    let thumbs = detect_thumbs();
    let size = terminal.size()?;

    let (tx, mut rx) = unbounded_channel();
    let env = Arc::new(Env::new(tools, caps.clone()));
    let mut runner = EffectRunner::new(env, tx);
    let (mut app, effects) = App::new(AppInit {
        dir: opts.dir,
        files: opts.files,
        caps,
        thumbs,
        size: (size.width, size.height),
    });
    for e in effects {
        runner.run(e);
    }

    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    #[cfg(unix)]
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut quit_deadline: Option<tokio::time::Instant> = None;
    let mut first_frame = true;

    loop {
        terminal.draw(|f| view::draw(&app, f))?;
        if first_frame {
            first_frame = false;
            // Gancho de teste: prova que o terminal é restaurado após um pânico.
            #[cfg(debug_assertions)]
            if std::env::var_os("NAGO_TUI_TEST_PANIC").is_some() {
                panic!("pânico de teste (NAGO_TUI_TEST_PANIC)");
            }
        }
        let deadline = app.next_deadline();
        let msg = tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(Event::Key(k))) if k.kind == KeyEventKind::Press => Msg::Key(k, Instant::now()),
                Some(Ok(Event::Resize(w, h))) => Msg::Resize(w, h),
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(e),
                None => Msg::Terminate,
            },
            Some(m) = rx.recv() => m,
            _ = tick.tick() => Msg::Tick(Instant::now()),
            _ = sleep_until(deadline), if deadline.is_some() => Msg::Tick(Instant::now()),
            _ = sigterm_recv(#[cfg(unix)] &mut sigterm) => Msg::Terminate,
            _ = tokio::time::sleep_until(quit_deadline.unwrap_or_else(tokio::time::Instant::now)),
                if quit_deadline.is_some() => {
                return Ok(TuiExit { code: app.quitting.unwrap_or(130) });
            }
        };
        for effect in update(&mut app, msg) {
            match effect {
                Effect::Exit(code) => return Ok(TuiExit { code }),
                other => runner.run(other),
            }
        }
        if app.quitting.is_some() && quit_deadline.is_none() {
            quit_deadline = Some(tokio::time::Instant::now() + QUIT_GRACE);
        }
    }
}

async fn sleep_until(deadline: Option<Instant>) {
    if let Some(d) = deadline {
        tokio::time::sleep_until(tokio::time::Instant::from_std(d)).await;
    }
}

#[cfg(unix)]
async fn sigterm_recv(sig: &mut tokio::signal::unix::Signal) {
    sig.recv().await;
}

#[cfg(not(unix))]
async fn sigterm_recv() {
    std::future::pending::<()>().await;
}
