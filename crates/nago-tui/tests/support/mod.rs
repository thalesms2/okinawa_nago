//! Utilitários dos testes da TUI: app sem terminal, teclas, relógio e tela em texto.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nago_core::caps::Capabilities;
use nago_tui::app::{App, AppInit, Effect, Msg, ThumbSupport, update};
use nago_tui::browser::Entry;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// Capacidades simuladas. `working_hw` lista os encoders de hardware que funcionam.
pub fn caps(working_hw: &[&str]) -> Capabilities {
    let hw: Vec<serde_json::Value> = ["av1_nvenc", "hevc_nvenc", "h264_nvenc"]
        .iter()
        .map(|e| {
            let ok = working_hw.contains(e);
            serde_json::json!({
                "encoder": e,
                "working": ok,
                "reason": if ok { None } else { Some("dispositivo indisponível") },
            })
        })
        .collect();
    serde_json::from_value(serde_json::json!({
        "key": {"format": nago_core::caps::CACHE_FORMAT, "ffmpeg": "/usr/bin/ffmpeg", "mtime_secs": 1, "version": "9.0.2"},
        "compiled": ["libsvtav1", "libx265", "libx264", "av1_nvenc", "hevc_nvenc", "h264_nvenc",
                     "aac", "libopus", "libmp3lame", "flac", "alac", "mov_text", "webvtt", "srt", "subrip"],
        "vmaf": true,
        "zscale": true,
        "hardware": hw,
    }))
    .unwrap()
}

/// Relógio de teste: cada chamada avança o tempo.
pub struct Clock {
    pub now: Instant,
}

impl Clock {
    pub fn new() -> Clock {
        Clock {
            now: Instant::now(),
        }
    }

    pub fn advance(&mut self, ms: u64) -> Instant {
        self.now += Duration::from_millis(ms);
        self.now
    }
}

pub struct Harness {
    pub app: App,
    pub clock: Clock,
}

pub fn entry(dir: &Path, name: &str, is_dir: bool) -> Entry {
    Entry {
        name: name.into(),
        path: dir.join(name),
        is_dir,
    }
}

impl Harness {
    pub fn with(
        dir: &Path,
        files: Vec<PathBuf>,
        caps: Capabilities,
        thumbs: Option<ThumbSupport>,
    ) -> (Harness, Vec<Effect>) {
        let (app, effects) = App::new(AppInit {
            dir: dir.to_path_buf(),
            files,
            caps,
            thumbs,
            size: (120, 30),
        });
        (
            Harness {
                app,
                clock: Clock::new(),
            },
            effects,
        )
    }

    /// App num diretório virtual já listado.
    pub fn listed(dir: &Path, entries: Vec<Entry>) -> (Harness, Vec<Effect>) {
        let (mut h, _) = Harness::with(dir, Vec::new(), caps(&[]), None);
        let effects = h.send(Msg::DirListed {
            dir: dir.to_path_buf(),
            result: Ok(entries),
        });
        (h, effects)
    }

    pub fn send(&mut self, msg: Msg) -> Vec<Effect> {
        update(&mut self.app, msg)
    }

    pub fn key(&mut self, code: KeyCode) -> Vec<Effect> {
        self.key_mod(code, KeyModifiers::NONE)
    }

    pub fn key_mod(&mut self, code: KeyCode, m: KeyModifiers) -> Vec<Effect> {
        let now = self.clock.advance(10);
        update(&mut self.app, Msg::Key(KeyEvent::new(code, m), now))
    }

    pub fn char(&mut self, c: char) -> Vec<Effect> {
        self.key(KeyCode::Char(c))
    }

    pub fn type_str(&mut self, s: &str) -> Vec<Effect> {
        s.chars().flat_map(|c| self.char(c)).collect()
    }

    /// Avança o relógio e entrega um tick (dispara efeitos adiados).
    pub fn tick(&mut self, ms: u64) -> Vec<Effect> {
        let now = self.clock.advance(ms);
        update(&mut self.app, Msg::Tick(now))
    }

    pub fn screen(&self) -> String {
        render(&self.app, 120, 30)
    }
}

/// Desenha o app num terminal de teste e devolve as linhas como texto.
pub fn render(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| nago_tui::view::draw(app, f)).unwrap();
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut line = String::new();
        for x in 0..buf.area.width {
            line.push_str(buf[(x, y)].symbol());
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

pub fn probes(effects: &[Effect]) -> Vec<PathBuf> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Probe(p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}
