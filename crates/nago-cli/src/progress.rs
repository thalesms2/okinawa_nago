//! Renderização dos eventos do core: barra de progresso (só em TTY) e avisos.

use std::io::IsTerminal;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use nago_core::runner::JobEvent;
use nago_core::timefmt;
use tokio::sync::mpsc::UnboundedReceiver;

/// Consome eventos até o canal fechar.
pub async fn render(mut rx: UnboundedReceiver<JobEvent>) {
    let interactive = std::io::stderr().is_terminal();
    let mut bar: Option<ProgressBar> = None;

    while let Some(event) = rx.recv().await {
        match event {
            JobEvent::Phase { name } => {
                if let Some(b) = bar.take() {
                    b.finish_and_clear();
                }
                if interactive {
                    let b = ProgressBar::with_draw_target(Some(1000), ProgressDrawTarget::stderr());
                    b.set_style(
                        ProgressStyle::with_template(
                            "{msg:>16} [{bar:32.cyan/blue}] {percent:>3}%  {prefix}",
                        )
                        .unwrap()
                        .progress_chars("━╸ "),
                    );
                    b.set_message(name);
                    b.enable_steady_tick(Duration::from_millis(250));
                    bar = Some(b);
                }
            }
            JobEvent::Progress(p) => {
                if let Some(b) = &bar {
                    b.set_position((p.ratio * 1000.0).round() as u64);
                    let speed = p
                        .speed
                        .map(|s| format!("{s:.1}x"))
                        .unwrap_or_else(|| "–".into());
                    let eta = p
                        .eta_secs
                        .map(|e| {
                            timefmt::format(e)
                                .split('.')
                                .next()
                                .unwrap_or("")
                                .to_owned()
                        })
                        .unwrap_or_else(|| "--:--:--".into());
                    b.set_prefix(format!("{speed}  ETA {eta}"));
                }
            }
            JobEvent::Warning(msg) => match &bar {
                Some(b) => b.suspend(|| eprintln!("aviso: {msg}")),
                None => eprintln!("aviso: {msg}"),
            },
        }
    }
    if let Some(b) = bar.take() {
        b.finish_and_clear();
    }
}
