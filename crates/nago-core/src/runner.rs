//! Execução de processos do ffmpeg com progresso, cancelamento e saída atômica
//! (spec `job-execution`, design D3 e D9).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::tools::Tools;
use crate::{Error, Result};

/// Eventos emitidos durante a execução; a CLI/TUI decide como mostrá-los.
#[derive(Debug, Clone, PartialEq)]
pub enum JobEvent {
    /// Uma nova fase começou (ex.: "codificando", "medindo VMAF").
    Phase {
        name: String,
    },
    Progress(Progress),
    Warning(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    /// 0.0..=1.0
    pub ratio: f64,
    /// Velocidade relativa ao tempo real (`3.2` = 3,2x).
    pub speed: Option<f64>,
    pub eta_secs: Option<f64>,
    pub out_time_secs: f64,
}

/// Quantas linhas finais do stderr guardar para mensagens de erro.
const STDERR_TAIL: usize = 50;

/// Executa invocações do ffmpeg, encaminhando eventos e respeitando o cancelamento.
#[derive(Clone)]
pub struct Runner {
    tools: Tools,
    events: Option<UnboundedSender<JobEvent>>,
    cancel: CancellationToken,
    /// Diretório onde salvar o log completo do ffmpeg (`--verbose`).
    log_dir: Option<PathBuf>,
}

impl Runner {
    pub fn new(tools: Tools) -> Runner {
        Runner {
            tools,
            events: None,
            cancel: CancellationToken::new(),
            log_dir: None,
        }
    }

    pub fn with_events(mut self, tx: UnboundedSender<JobEvent>) -> Runner {
        self.events = Some(tx);
        self
    }

    pub fn with_cancel(mut self, token: CancellationToken) -> Runner {
        self.cancel = token;
        self
    }

    pub fn with_log_dir(mut self, dir: Option<PathBuf>) -> Runner {
        self.log_dir = dir;
        self
    }

    pub fn tools(&self) -> &Tools {
        &self.tools
    }

    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel
    }

    pub fn emit(&self, event: JobEvent) {
        if let Some(tx) = &self.events {
            let _ = tx.send(event);
        }
    }

    pub fn warn(&self, msg: impl Into<String>) {
        self.emit(JobEvent::Warning(msg.into()));
    }

    pub fn check_cancelled(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Executa o ffmpeg com `args` (entradas, mapeamentos e saída) e acompanha o progresso
    /// em relação a `expected_duration` segundos de mídia.
    pub async fn run_ffmpeg(
        &self,
        phase: &str,
        args: &[OsString],
        expected_duration: f64,
    ) -> Result<()> {
        self.check_cancelled()?;
        self.emit(JobEvent::Phase {
            name: phase.to_owned(),
        });

        let verbose = self.log_dir.is_some();
        let mut cmd = tokio::process::Command::new(&self.tools.ffmpeg);
        cmd.args([
            "-hide_banner",
            "-nostats",
            "-progress",
            "pipe:1",
            "-y",
            "-loglevel",
        ])
        .arg(if verbose { "info" } else { "warning" })
        .args(args)
        // SVT-AV1 escreve no stderr independentemente do -loglevel.
        .env("SVT_LOG", if verbose { "3" } else { "2" })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
        // Grupo de processos próprio: o Ctrl+C do terminal chega só ao nago,
        // que encerra o ffmpeg de forma controlada.
        #[cfg(unix)]
        cmd.process_group(0);

        tracing::debug!(?args, "executando ffmpeg");
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Ffmpeg(format!("falha ao iniciar o ffmpeg: {e}")))?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let mut stdin = child.stdin.take();

        let progress_task = {
            let runner = self.clone();
            tokio::spawn(async move {
                let mut parser = ProgressParser::new(expected_duration);
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(p) = parser.feed(&line) {
                        runner.emit(JobEvent::Progress(p));
                    }
                }
            })
        };

        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL)));
        let log_file = self.log_dir.as_ref().map(|dir| log_path(dir, phase));
        let stderr_task = {
            let tail = Arc::clone(&tail);
            let log_file = log_file.clone();
            tokio::spawn(async move {
                let mut file = match &log_file {
                    Some(p) => {
                        if let Some(parent) = p.parent() {
                            let _ = tokio::fs::create_dir_all(parent).await;
                        }
                        tokio::fs::File::create(p).await.ok()
                    }
                    None => None,
                };
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(f) = file.as_mut() {
                        let _ = f.write_all(line.as_bytes()).await;
                        let _ = f.write_all(b"\n").await;
                    }
                    let mut t = tail.lock().unwrap();
                    if t.len() == STDERR_TAIL {
                        t.pop_front();
                    }
                    t.push_back(line);
                }
            })
        };

        let status = tokio::select! {
            status = child.wait() => status?,
            () = self.cancel.cancelled() => {
                stop_gracefully(&mut child, stdin.take()).await;
                let _ = progress_task.await;
                let _ = stderr_task.await;
                return Err(Error::Cancelled);
            }
        };
        drop(stdin);
        let _ = progress_task.await;
        let _ = stderr_task.await;
        self.check_cancelled()?;

        if status.success() {
            return Ok(());
        }
        let tail: Vec<String> = tail.lock().unwrap().iter().cloned().collect();
        Err(Error::FfmpegFailed {
            summary: match status.code() {
                Some(code) => format!("o ffmpeg falhou ({phase}, código {code})"),
                None => format!("o ffmpeg foi encerrado por um sinal ({phase})"),
            },
            log_tail: relevant_tail(&tail, 8),
            log_file,
        })
    }
}

/// `q` no stdin → SIGTERM após 2 s → SIGKILL após mais 3 s.
async fn stop_gracefully(
    child: &mut tokio::process::Child,
    stdin: Option<tokio::process::ChildStdin>,
) {
    if let Some(mut stdin) = stdin {
        let _ = stdin.write_all(b"q").await;
        let _ = stdin.flush().await;
    }
    if tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .is_ok()
    {
        return;
    }
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        use nix::sys::signal::{Signal, kill};
        use nix::unistd::Pid;
        let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
        if tokio::time::timeout(Duration::from_secs(3), child.wait())
            .await
            .is_ok()
        {
            return;
        }
    }
    let _ = child.kill().await;
}

fn log_path(dir: &Path, phase: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let phase: String = phase
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    dir.join(format!("ffmpeg-{stamp}-{phase}.log"))
}

/// Últimas `n` linhas não vazias, sem as linhas de configuração do SVT-AV1.
fn relevant_tail(lines: &[String], n: usize) -> Vec<String> {
    let relevant: Vec<&String> = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .filter(|l| !l.starts_with("Svt[info]") && !l.starts_with("SvtMalloc"))
        .collect();
    relevant[relevant.len().saturating_sub(n)..]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Progresso

/// Interpreta a saída de `-progress pipe:1` (pares `chave=valor` em blocos
/// terminados por `progress=continue|end`).
#[derive(Debug)]
pub struct ProgressParser {
    expected: f64,
    out_time: Option<f64>,
    speed: Option<f64>,
}

impl ProgressParser {
    pub fn new(expected_duration: f64) -> ProgressParser {
        ProgressParser {
            expected: expected_duration,
            out_time: None,
            speed: None,
        }
    }

    /// Alimenta uma linha; devolve uma amostra ao final de cada bloco.
    pub fn feed(&mut self, line: &str) -> Option<Progress> {
        let (key, value) = line.split_once('=')?;
        let value = value.trim();
        match key.trim() {
            "out_time_us" => {
                if let Ok(us) = value.parse::<i64>() {
                    self.out_time = Some(us.max(0) as f64 / 1e6);
                }
            }
            "speed" => {
                self.speed = value
                    .trim_end_matches('x')
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|s| *s > 0.0 && s.is_finite());
            }
            "progress" => {
                let done = value == "end";
                let out_time = self.out_time.unwrap_or(0.0);
                let ratio = if done {
                    1.0
                } else if self.expected > 0.0 {
                    (out_time / self.expected).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let eta_secs = if done {
                    Some(0.0)
                } else {
                    self.speed
                        .map(|s| ((self.expected - out_time).max(0.0)) / s)
                };
                return Some(Progress {
                    ratio,
                    speed: self.speed,
                    eta_secs,
                    out_time_secs: out_time,
                });
            }
            _ => {}
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Saída atômica

/// Arquivo temporário no diretório de destino, removido no `Drop` a menos que
/// seja promovido com [`TempOutput::persist`].
#[derive(Debug)]
pub struct TempOutput {
    temp: tempfile::TempPath,
    final_path: PathBuf,
}

impl TempOutput {
    /// Cria `<dir>/.<nome>.nago-XXXXXX.part.<ext>` ao lado de `final_path`.
    pub fn new(final_path: &Path) -> Result<TempOutput> {
        let dir = final_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = final_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "saida".into());
        let ext = final_path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let temp = tempfile::Builder::new()
            .prefix(&format!(".{name}.nago-"))
            .suffix(&format!(".part{ext}"))
            .tempfile_in(dir)
            .map_err(|e| {
                Error::Other(format!(
                    "não foi possível criar arquivo temporário em {}: {e}",
                    dir.display()
                ))
            })?
            .into_temp_path();
        Ok(TempOutput {
            temp,
            final_path: final_path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.temp
    }

    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    pub fn size(&self) -> u64 {
        std::fs::metadata(&*self.temp).map(|m| m.len()).unwrap_or(0)
    }

    /// Renomeia atomicamente para o caminho final, com as permissões de `like`
    /// (o temporário nasce com 0600; a saída deve ficar como um arquivo comum).
    pub fn persist_like(self, like: &Path) -> Result<PathBuf> {
        if let Ok(meta) = std::fs::metadata(like) {
            let _ = std::fs::set_permissions(&*self.temp, meta.permissions());
        }
        self.persist()
    }

    /// Renomeia atomicamente para o caminho final.
    pub fn persist(self) -> Result<PathBuf> {
        let final_path = self.final_path.clone();
        self.temp.persist(&final_path).map_err(|e| {
            Error::Other(format!(
                "não foi possível gravar {}: {}",
                final_path.display(),
                e.error
            ))
        })?;
        Ok(final_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_de_progresso_real() {
        let text = include_str!("../tests/data/progress_sample.txt");
        let mut p = ProgressParser::new(2.0);
        let samples: Vec<Progress> = text.lines().filter_map(|l| p.feed(l)).collect();
        let last = samples.last().unwrap();
        assert_eq!(last.ratio, 1.0);
        assert_eq!(last.out_time_secs, 2.0);
        assert_eq!(last.speed, Some(205.0));
    }

    #[test]
    fn progresso_crescente_com_eta() {
        let text = "out_time_us=2500000\nspeed=2.5x\nprogress=continue\n\
                    out_time_us=N/A\nspeed=N/A\nprogress=continue\n\
                    out_time_us=5000000\nspeed=   2x\nprogress=continue\n";
        let mut p = ProgressParser::new(10.0);
        let s: Vec<Progress> = text.lines().filter_map(|l| p.feed(l)).collect();
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].ratio, 0.25);
        assert_eq!(s[0].eta_secs, Some(3.0));
        // N/A mantém o último out_time e perde a velocidade
        assert_eq!(s[1].ratio, 0.25);
        assert_eq!(s[1].eta_secs, None);
        assert_eq!(s[2].ratio, 0.5);
        assert_eq!(s[2].eta_secs, Some(2.5));
    }

    #[test]
    fn cauda_relevante_ignora_svt() {
        let lines: Vec<String> = ["Svt[info]: x", "", "erro A", "SvtMalloc[info]: y", "erro B"]
            .map(String::from)
            .to_vec();
        assert_eq!(relevant_tail(&lines, 8), ["erro A", "erro B"]);
    }

    #[test]
    fn temp_output_e_removido_no_drop_e_persistido_no_sucesso() {
        let dir = tempfile::tempdir().unwrap();
        let final_path = dir.path().join("saida.mp4");
        let t = TempOutput::new(&final_path).unwrap();
        let tmp = t.path().to_path_buf();
        let name = tmp.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with(".saida.mp4.nago-") && name.ends_with(".part.mp4"),
            "{name}"
        );
        assert!(tmp.exists());
        drop(t);
        assert!(!tmp.exists());

        let t = TempOutput::new(&final_path).unwrap();
        std::fs::write(t.path(), b"ok").unwrap();
        assert_eq!(t.persist().unwrap(), final_path);
        assert_eq!(std::fs::read(&final_path).unwrap(), b"ok");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn persist_like_copia_permissoes_da_entrada() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.mp4");
        std::fs::write(&input, b"x").unwrap();
        std::fs::set_permissions(&input, std::fs::Permissions::from_mode(0o644)).unwrap();
        let t = TempOutput::new(&dir.path().join("out.mp4")).unwrap();
        let out = t.persist_like(&input).unwrap();
        let mode = std::fs::metadata(out).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
    }
}
