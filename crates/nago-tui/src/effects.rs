//! Execução dos efeitos pedidos por `update` (todo o I/O da TUI fica aqui).
//!
//! Cada efeito roda numa task tokio e devolve o resultado como [`Msg`]. Probe,
//! validação e miniatura usam "vagas": um pedido novo aborta o anterior da mesma vaga
//! (o ffmpeg/ffprobe é encerrado por `kill_on_drop`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use nago_core::caps::Capabilities;
use nago_core::ops::{self, Prepared};
use nago_core::output::Reservations;
use nago_core::preview;
use nago_core::probe::{self, MediaInfo};
use nago_core::runner::Runner;
use nago_core::tools::Tools;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::app::{Effect, Msg};
use crate::browser;
use crate::opform::{OpRequest, Summary};
use crate::queue::{JobFailure, JobId};

type Stamp = (Option<SystemTime>, u64);

fn stamp(path: &Path) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok(), m.len()))
}

/// Ferramentas, capacidades e o cache de probes da sessão.
pub struct Env {
    pub tools: Tools,
    pub caps: Capabilities,
    probes: Mutex<HashMap<PathBuf, (Stamp, MediaInfo)>>,
}

impl Env {
    pub fn new(tools: Tools, caps: Capabilities) -> Env {
        Env {
            tools,
            caps,
            probes: Mutex::new(HashMap::new()),
        }
    }

    /// Probe com cache por (caminho, mtime, tamanho).
    pub async fn probe(&self, path: &Path) -> nago_core::Result<MediaInfo> {
        let current = stamp(path);
        if let Some(st) = current
            && let Some((cached, info)) = self.probes.lock().ok().and_then(|m| m.get(path).cloned())
            && cached == st
        {
            return Ok(info);
        }
        let info = probe::probe(&self.tools, path).await?;
        if let (Some(st), Ok(mut m)) = (current, self.probes.lock()) {
            m.insert(path.to_path_buf(), (st, info.clone()));
        }
        Ok(info)
    }

    pub async fn prepare(
        &self,
        request: &OpRequest,
        reserved: &Reservations,
    ) -> nago_core::Result<Prepared> {
        let info = self.probe(request.input()).await?;
        let (tools, caps) = (&self.tools, &self.caps);
        match request {
            OpRequest::Compress(r) => {
                ops::prepare_compress(tools, caps, r, Some(&info), reserved).await
            }
            OpRequest::Convert(r) => {
                ops::prepare_convert(tools, caps, r, Some(&info), reserved).await
            }
            OpRequest::Cut(r) => ops::prepare_cut(tools, caps, r, Some(&info), reserved).await,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Slot {
    Probe,
    Prepare,
    Frame,
}

pub struct EffectRunner {
    env: Arc<Env>,
    tx: UnboundedSender<Msg>,
    slots: HashMap<Slot, JoinHandle<()>>,
    jobs: HashMap<JobId, CancellationToken>,
}

impl EffectRunner {
    pub fn new(env: Arc<Env>, tx: UnboundedSender<Msg>) -> EffectRunner {
        EffectRunner {
            env,
            tx,
            slots: HashMap::new(),
            jobs: HashMap::new(),
        }
    }

    fn spawn_in(&mut self, slot: Slot, fut: impl Future<Output = ()> + Send + 'static) {
        if let Some(old) = self.slots.insert(slot, tokio::spawn(fut)) {
            old.abort();
        }
    }

    /// Executa um efeito. `Exit` é tratado pelo laço principal e ignorado aqui.
    pub fn run(&mut self, effect: Effect) {
        let env = self.env.clone();
        let tx = self.tx.clone();
        match effect {
            Effect::Exit(_) => {}
            Effect::ListDir(dir) => {
                tokio::spawn(async move {
                    let d = dir.clone();
                    let result = tokio::task::spawn_blocking(move || browser::read_dir(&d))
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r.map_err(|e| format!("não foi possível ler a pasta: {e}")));
                    let _ = tx.send(Msg::DirListed { dir, result });
                });
            }
            Effect::Probe(path) => self.spawn_in(Slot::Probe, async move {
                let result = env.probe(&path).await.map_err(|e| e.to_string());
                let _ = tx.send(Msg::Probed { path, result });
            }),
            Effect::Prepare {
                seq,
                request,
                reserved,
            } => self.spawn_in(Slot::Prepare, async move {
                let result = env
                    .prepare(&request, &reserved)
                    .await
                    .map(|p| Summary::from(&p))
                    .map_err(|e| e.to_string());
                let _ = tx.send(Msg::Prepared { seq, result });
            }),
            Effect::Enqueue {
                requests,
                mut reserved,
            } => {
                tokio::spawn(async move {
                    let mut results = Vec::new();
                    for request in requests {
                        let result = match env.prepare(&request, &reserved).await {
                            Ok(p) => {
                                reserved.insert(&p.output);
                                Ok(p.output)
                            }
                            Err(e) => Err(e.to_string()),
                        };
                        results.push((request, result));
                    }
                    let _ = tx.send(Msg::Enqueued { results });
                });
            }
            Effect::IndexKeyframes(path) => {
                tokio::spawn(async move {
                    let result = match env.probe(&path).await {
                        Ok(info) => preview::keyframe_index(&env.tools, &info).await,
                        Err(e) => Err(e),
                    }
                    .map_err(|e| e.to_string());
                    let _ = tx.send(Msg::KeyframesIndexed { path, result });
                });
            }
            Effect::Frame { seq, path, t, key } => self.spawn_in(Slot::Frame, async move {
                let (_, w, h) = key;
                let result = async {
                    let info = env.probe(&path).await.map_err(|e| e.to_string())?;
                    let png = preview::frame_png(&env.tools, &info, t, w, h, env.caps.zscale)
                        .await
                        .map_err(|e| e.to_string())?;
                    tokio::task::spawn_blocking(move || image::load_from_memory(&png))
                        .await
                        .map_err(|e| e.to_string())?
                        .map_err(|e| format!("frame inválido: {e}"))
                }
                .await;
                let _ = tx.send(Msg::Frame { seq, key, result });
            }),
            Effect::StartJob {
                id,
                request,
                reserved,
            } => {
                let token = CancellationToken::new();
                self.jobs.insert(id, token.clone());
                tokio::spawn(run_job(env, tx, id, request, reserved, token));
            }
            Effect::CancelJob(id) => {
                if let Some(token) = self.jobs.get(&id) {
                    token.cancel();
                }
            }
        }
    }
}

async fn run_job(
    env: Arc<Env>,
    tx: UnboundedSender<Msg>,
    id: JobId,
    request: OpRequest,
    reserved: Reservations,
    token: CancellationToken,
) {
    let finish = |result| {
        let _ = tx.send(Msg::JobFinished { id, result });
    };
    let prepared = match env.prepare(&request, &reserved).await {
        Ok(p) => p,
        Err(e) => return finish(Err(JobFailure::from_error(&e))),
    };
    if token.is_cancelled() {
        return finish(Err(JobFailure::from_error(&nago_core::Error::Cancelled)));
    }
    let _ = tx.send(Msg::JobStarted {
        id,
        output: prepared.output.clone(),
    });
    let (etx, mut erx) = unbounded_channel();
    let forward_tx = tx.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(event) = erx.recv().await {
            let _ = forward_tx.send(Msg::Job { id, event });
        }
    });
    let result = {
        let runner = Runner::new(env.tools.clone())
            .with_events(etx)
            .with_cancel(token);
        ops::execute(&runner, &prepared).await
    };
    // o runner (e o transmissor de eventos) já foi descartado: o repassador termina
    let _ = forwarder.await;
    finish(result.map_err(|e| JobFailure::from_error(&e)));
}
