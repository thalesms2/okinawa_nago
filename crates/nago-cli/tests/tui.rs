//! TUI num pseudo-terminal real (specs `interactive-mode` e `job-queue`).
#![cfg(unix)]

use std::io::{Read, Write};
use std::num::NonZero;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const LEAVE_ALT_SCREEN: &[u8] = b"\x1b[?1049l";

/// Uma sessão do `nago` num pty, com a tela interpretada por um emulador vt100.
struct Session {
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    _master: Box<dyn MasterPty + Send>,
    screen: Arc<Mutex<vt100::Parser>>,
    raw: Arc<Mutex<Vec<u8>>>,
}

impl Session {
    fn spawn(args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> Session {
        let pty = native_pty_system()
            .openpty(PtySize {
                rows: 30,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = CommandBuilder::new(assert_cmd::cargo::cargo_bin("nago"));
        cmd.args(args);
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("NAGO_TUI_IMAGES", "off");
        cmd.env(
            "NAGO_CACHE_DIR",
            std::env::temp_dir().join("nago-test-cache"),
        );
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = pty.slave.spawn_command(cmd).unwrap();
        drop(pty.slave);
        let mut reader = pty.master.try_clone_reader().unwrap();
        let writer = pty.master.take_writer().unwrap();
        let screen = Arc::new(Mutex::new(vt100::Parser::new(
            NonZero::new(30).unwrap(),
            NonZero::new(120).unwrap(),
            0,
        )));
        let raw = Arc::new(Mutex::new(Vec::new()));
        let (s, r) = (screen.clone(), raw.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                s.lock().unwrap().process(&buf[..n]);
                r.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        Session {
            child,
            writer,
            _master: pty.master,
            screen,
            raw,
        }
    }

    fn contents(&self) -> String {
        self.screen.lock().unwrap().screen().contents()
    }

    fn raw(&self) -> Vec<u8> {
        self.raw.lock().unwrap().clone()
    }

    /// Espera o texto aparecer na tela.
    fn wait_for(&self, text: &str, timeout: Duration) {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.contents().contains(text) {
                return;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        panic!("`{text}` não apareceu na tela:\n{}", self.contents());
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).unwrap();
        self.writer.flush().unwrap();
    }

    fn pid(&self) -> i32 {
        self.child.process_id().unwrap() as i32
    }

    fn wait_exit(&mut self, timeout: Duration) -> u32 {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Some(status) = self.child.try_wait().unwrap() {
                // dá tempo ao leitor de consumir o fim da saída
                std::thread::sleep(Duration::from_millis(100));
                return status.exit_code();
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let _ = self.child.kill();
        panic!("o nago não terminou:\n{}", self.contents());
    }
}

fn workdir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    nago_testkit::fixture_in(nago_testkit::Fixture::H264Aac360p, dir.path());
    std::fs::write(dir.path().join("notas.txt"), "x").unwrap();
    dir
}

#[test]
fn nago_sem_argumentos_abre_a_tela_inicial_e_sai_com_q() {
    let dir = workdir();
    let mut s = Session::spawn(&[], dir.path(), &[]);
    s.wait_for("arquivos de mídia", Duration::from_secs(20));
    s.wait_for("h264_aac_360p.mp4", Duration::from_secs(5));
    assert!(!s.contents().contains("notas.txt"));
    // o painel de metadados lê o arquivo destacado
    s.wait_for("640x360", Duration::from_secs(10));
    s.send(b"q");
    assert_eq!(s.wait_exit(Duration::from_secs(10)), 0);
    assert!(
        s.raw()
            .windows(LEAVE_ALT_SCREEN.len())
            .any(|w| w == LEAVE_ALT_SCREEN),
        "o terminal não saiu da tela alternativa"
    );
}

#[test]
fn tui_com_arquivos_abre_com_eles_marcados() {
    let dir = workdir();
    let second = nago_testkit::fixture_in(nago_testkit::Fixture::MkvMulti, dir.path());
    let first = dir.path().join("h264_aac_360p.mp4");
    let mut s = Session::spawn(
        &["tui", first.to_str().unwrap(), second.to_str().unwrap()],
        dir.path(),
        &[],
    );
    s.wait_for("[x] h264_aac_360p.mp4", Duration::from_secs(20));
    s.wait_for("[x] multi_tracks.mkv", Duration::from_secs(5));
    // metadados do primeiro arquivo
    s.wait_for("640x360", Duration::from_secs(10));
    s.send(b"q");
    assert_eq!(s.wait_exit(Duration::from_secs(10)), 0);
}

#[test]
fn panico_restaura_o_terminal_e_sai_com_1() {
    let dir = workdir();
    let mut s = Session::spawn(&[], dir.path(), &[("NAGO_TUI_TEST_PANIC", "1")]);
    assert_eq!(s.wait_exit(Duration::from_secs(20)), 1);
    let raw = s.raw();
    let leave = raw
        .windows(LEAVE_ALT_SCREEN.len())
        .position(|w| w == LEAVE_ALT_SCREEN)
        .expect("o terminal não saiu da tela alternativa");
    let text = String::from_utf8_lossy(&raw[leave..]);
    assert!(text.contains("pânico de teste"), "{text}");
    assert!(text.contains("erro interno"), "{text}");
}

fn files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// Comprime pela TUI (z, Enter) e manda SIGTERM no meio: código 130, sem parcial.
#[test]
fn sigterm_durante_a_compressao_sai_com_130_sem_arquivo_parcial() {
    let dir = tempfile::tempdir().unwrap();
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=30:duration=60",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
        ])
        .arg(dir.path().join("longo.mp4"))
        .status()
        .unwrap();
    assert!(ok.success());
    let mut s = Session::spawn(&[], dir.path(), &[]);
    s.wait_for("longo.mp4", Duration::from_secs(20));
    s.wait_for("1280x720", Duration::from_secs(10));
    s.send(b"z");
    s.wait_for("encoder: libsvtav1", Duration::from_secs(20));
    s.send(b"\r");
    s.wait_for("1 job na fila", Duration::from_secs(20));
    s.send(b"\t");
    s.wait_for("codificando", Duration::from_secs(20));
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(s.pid()),
        nix::sys::signal::Signal::SIGTERM,
    )
    .unwrap();
    assert_eq!(s.wait_exit(Duration::from_secs(20)), 130);
    assert_eq!(files(dir.path()), ["longo.mp4"]);
}
