//! Spec `job-queue`: fila sequencial, estados, relatório e cancelamento.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::KeyCode;
use nago_core::encoders::Codec;
use nago_core::ops::{CompressRequest, ConvertRequest, CutRequest, OutputOptions};
use nago_core::plan::{self, CompressOptions};
use nago_core::probe;
use nago_core::profiles::Quality;
use nago_core::report::Report;
use nago_core::runner::{JobEvent, Progress};
use nago_core::tools::Tools;
use nago_testkit::{Fixture, fixture_in};
use nago_tui::app::{App, Effect, Msg, Screen, update};
use nago_tui::effects::{EffectRunner, Env};
use nago_tui::opform::OpRequest;
use nago_tui::queue::{FailKind, JobFailure, JobState};
use support::{Harness, caps};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

fn compress(input: &Path, quality: Quality) -> OpRequest {
    OpRequest::Compress(CompressRequest {
        input: input.to_path_buf(),
        output: OutputOptions::default(),
        quality,
        codec: Codec::Av1,
        gpu: false,
        audio_bitrate: None,
        verify: false,
        force: true,
    })
}

fn cut(input: &Path) -> OpRequest {
    OpRequest::Cut(CutRequest {
        input: input.to_path_buf(),
        output: OutputOptions::default(),
        from: Some(1.0),
        to: Some(3.0),
        duration: None,
        exact: false,
    })
}

fn convert(input: &Path, to: &str) -> OpRequest {
    OpRequest::Convert(ConvertRequest {
        input: input.to_path_buf(),
        output: OutputOptions::default(),
        format: Some(to.into()),
        audio_track: None,
        compat: false,
    })
}

fn harness() -> Harness {
    Harness::with(Path::new("/v"), vec![], caps(&[]), None).0
}

fn enqueue(h: &mut Harness, request: OpRequest, output: &str) -> Vec<Effect> {
    h.send(Msg::Enqueued {
        results: vec![(request, Ok(PathBuf::from(output)))],
    })
}

fn fail(kind: FailKind) -> JobFailure {
    JobFailure {
        kind,
        message: "x".into(),
        log_tail: vec![],
    }
}

fn starts(effects: &[Effect]) -> Vec<u64> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::StartJob { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Política da fila (sem executar nada)

#[test]
fn enfileirar_durante_a_execucao() {
    let mut h = harness();
    let e = enqueue(
        &mut h,
        compress(Path::new("/v/a.mp4"), Quality::High),
        "/v/a.compressed.mp4",
    );
    assert_eq!(starts(&e), [1]);
    h.send(Msg::JobStarted {
        id: 1,
        output: "/v/a.compressed.mp4".into(),
    });
    let e = enqueue(&mut h, cut(Path::new("/v/b.mp4")), "/v/b.cut.mp4");
    assert!(starts(&e).is_empty(), "um job por vez");
    assert_eq!(h.app.queue.get(2).unwrap().state, JobState::Pending);
    // o usuário continua navegando enquanto a fila roda
    assert_eq!(h.app.screen, Screen::Browser);
    let e = h.send(Msg::JobFinished {
        id: 1,
        result: Err(fail(FailKind::Failed)),
    });
    assert_eq!(starts(&e), [2]);
}

#[test]
fn reservas_excluem_o_proprio_job_ao_comecar() {
    let mut h = harness();
    enqueue(
        &mut h,
        compress(Path::new("/v/a.mp4"), Quality::High),
        "/v/a.compressed.mp4",
    );
    let e = enqueue(
        &mut h,
        compress(Path::new("/v/a.mp4"), Quality::Small),
        "/v/a.compressed-1.mp4",
    );
    assert!(e.is_empty());
    let e = h.send(Msg::JobFinished {
        id: 1,
        result: Err(fail(FailKind::Discarded)),
    });
    let [
        Effect::StartJob {
            id: 2, reserved, ..
        },
    ] = &e[..]
    else {
        panic!("{e:?}");
    };
    // o job 1 terminou (descartado): nada mais reserva os dois nomes
    assert!(!reserved.contains(Path::new("/v/a.compressed-1.mp4")));
    assert!(!reserved.contains(Path::new("/v/a.compressed.mp4")));
}

#[test]
fn remover_pendente() {
    let mut h = harness();
    for (i, name) in ["a", "b", "c", "d"].iter().enumerate() {
        let input = PathBuf::from(format!("/v/{name}.mp4"));
        enqueue(&mut h, cut(&input), &format!("/v/{name}.cut.mp4"));
        if i == 0 {
            h.send(Msg::JobStarted {
                id: 1,
                output: "/v/a.cut.mp4".into(),
            });
        }
    }
    h.key(KeyCode::Tab);
    // seleciona o segundo pendente (job 3) e remove
    h.key(KeyCode::Down);
    h.key(KeyCode::Down);
    h.char('d');
    let ids: Vec<u64> = h.app.queue.jobs.iter().map(|j| j.id).collect();
    assert_eq!(ids, [1, 2, 4]);
    // o job em execução não é removido
    h.key(KeyCode::Up);
    h.key(KeyCode::Up);
    h.char('d');
    assert_eq!(h.app.queue.jobs.len(), 3);
    assert!(h.screen().contains("use x para cancelar"));
    let e = h.send(Msg::JobFinished {
        id: 1,
        result: Err(fail(FailKind::Cancelled)),
    });
    assert_eq!(starts(&e), [2]);
    let e = h.send(Msg::JobFinished {
        id: 2,
        result: Err(fail(FailKind::Failed)),
    });
    assert_eq!(starts(&e), [4]);
}

#[test]
fn cancelar_pela_tecla_x() {
    let mut h = harness();
    enqueue(&mut h, cut(Path::new("/v/a.mp4")), "/v/a.cut.mp4");
    h.key(KeyCode::Tab);
    let e = h.char('x');
    assert!(matches!(&e[..], [Effect::CancelJob(1)]));
}

// ---------------------------------------------------------------------------
// Execução real pelo EffectRunner

struct Driver {
    h: Harness,
    runner: EffectRunner,
    rx: UnboundedReceiver<Msg>,
}

impl Driver {
    async fn new(dir: &Path, tools: Tools) -> Driver {
        let (tx, rx) = unbounded_channel();
        let env = Arc::new(Env::new(tools, caps(&[])));
        let (h, _) = Harness::with(dir, vec![], caps(&[]), None);
        Driver {
            h,
            runner: EffectRunner::new(env, tx),
            rx,
        }
    }

    fn app(&self) -> &App {
        &self.h.app
    }

    fn run(&mut self, effects: Vec<Effect>) {
        for e in effects {
            self.runner.run(e);
        }
    }

    /// Processa mensagens até `done` ser verdadeiro.
    async fn until(&mut self, done: impl Fn(&App) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
        while !done(&self.h.app) {
            let msg = tokio::time::timeout_at(deadline, self.rx.recv())
                .await
                .expect("tempo esgotado esperando a fila")
                .expect("canal fechado");
            let effects = update(&mut self.h.app, msg);
            self.run(effects);
        }
    }

    async fn enqueue(&mut self, requests: Vec<OpRequest>) {
        let before = self.app().queue.jobs.len();
        let reserved = self.app().queue.reservations(None);
        self.run(vec![Effect::Enqueue { requests, reserved }]);
        self.until(|a| a.queue.jobs.len() > before).await;
    }
}

fn all_finished(app: &App) -> bool {
    app.queue.jobs.iter().all(|j| !j.state.is_active())
}

fn files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread")]
async fn tres_jobs_reais_em_sequencia() {
    let tmp = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, tmp.path());
    let mut d = Driver::new(tmp.path(), Tools::locate().await.unwrap()).await;
    d.enqueue(vec![
        cut(&input),
        convert(&input, "mkv"),
        compress(&input, Quality::Small),
    ])
    .await;
    assert_eq!(d.app().queue.jobs.len(), 3);
    d.until(all_finished).await;
    for job in &d.app().queue.jobs {
        assert!(
            matches!(job.state, JobState::Done(_)),
            "{}: {:?}",
            job.label,
            job.state
        );
    }
    assert_eq!(
        files(tmp.path()),
        [
            "h264_aac_360p.compressed.mp4",
            "h264_aac_360p.cut.mp4",
            "h264_aac_360p.mkv",
            "h264_aac_360p.mp4"
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mesmo_arquivo_comprimido_duas_vezes() {
    let tmp = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, tmp.path());
    let mut d = Driver::new(tmp.path(), Tools::locate().await.unwrap()).await;
    // dois envios separados, como dois formulários
    d.enqueue(vec![compress(&input, Quality::Small)]).await;
    d.enqueue(vec![compress(&input, Quality::Balanced)]).await;
    let planned: Vec<PathBuf> = d
        .app()
        .queue
        .jobs
        .iter()
        .map(|j| j.planned_output.clone())
        .collect();
    assert_eq!(
        planned,
        [
            tmp.path().join("h264_aac_360p.compressed.mp4"),
            tmp.path().join("h264_aac_360p.compressed-1.mp4")
        ]
    );
    d.until(all_finished).await;
    let outputs: Vec<PathBuf> = d
        .app()
        .queue
        .jobs
        .iter()
        .map(|j| j.output.clone().unwrap())
        .collect();
    assert_eq!(outputs, planned);
    assert!(outputs.iter().all(|p| p.exists()));
}

#[tokio::test(flavor = "multi_thread")]
async fn caminho_explicito_repetido_nao_enfileira() {
    let tmp = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, tmp.path());
    let mut d = Driver::new(tmp.path(), Tools::locate().await.unwrap()).await;
    let target = tmp.path().join("final.mkv");
    let with_out = |r: OpRequest| r.with_output(target.clone());
    d.enqueue(vec![with_out(convert(&input, "mkv"))]).await;
    // segundo formulário com a mesma saída: o core recusa e nada é enfileirado
    let reserved = d.app().queue.reservations(None);
    d.h.app.form = Some(nago_tui::opform::OpForm::new(
        nago_tui::opform::OpKind::Convert,
        vec![input.clone()],
    ));
    d.h.app.form.as_mut().unwrap().submitting = true;
    d.run(vec![Effect::Enqueue {
        requests: vec![with_out(convert(&input, "mkv"))],
        reserved,
    }]);
    d.until(|a| a.form.as_ref().is_some_and(|f| !f.submitting))
        .await;
    assert_eq!(d.app().queue.jobs.len(), 1);
    let form = d.app().form.as_ref().unwrap();
    let (field, msg) = form.form.first_error().unwrap();
    assert_eq!(field, nago_tui::form::FieldId::Output);
    assert!(msg.contains("outro job da fila"), "{msg}");
}

/// Compressão demorada o bastante para ser cancelada no meio.
fn slow_input(dir: &Path) -> PathBuf {
    let input = dir.join("longo.mp4");
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
        .arg(&input)
        .status()
        .unwrap();
    assert!(ok.success());
    input
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelar_o_job_atual_segue_para_o_proximo() {
    let tmp = tempfile::tempdir().unwrap();
    let input = slow_input(tmp.path());
    let mut d = Driver::new(tmp.path(), Tools::locate().await.unwrap()).await;
    let mut slow = compress(&input, Quality::VisuallyLossless);
    if let OpRequest::Compress(r) = &mut slow {
        r.codec = Codec::Hevc;
    }
    d.enqueue(vec![slow, cut(&input)]).await;
    // espera a codificação começar de fato
    d.until(|a| {
        matches!(
            &a.queue.jobs[0].state,
            JobState::Running {
                progress: Some(_),
                ..
            }
        )
    })
    .await;
    d.h.key(KeyCode::Tab);
    let effects = d.h.char('x');
    d.run(effects);
    d.until(all_finished).await;
    assert_eq!(d.app().queue.jobs[0].state, JobState::Cancelled);
    assert!(matches!(d.app().queue.jobs[1].state, JobState::Done(_)));
    // nenhum arquivo parcial (nem temporário oculto) da compressão cancelada
    assert_eq!(files(tmp.path()), ["longo.cut.mp4", "longo.mp4"]);
}

// ---------------------------------------------------------------------------
// Tela da fila

/// Relatório de compressão realista (sem executar nada).
async fn compress_report(input: &Path) -> Report {
    let tools = Tools::locate().await.unwrap();
    let info = probe::probe(&tools, input).await.unwrap();
    let opts = CompressOptions {
        quality: Quality::High,
        codec: Codec::Av1,
        gpu: false,
        audio_bitrate: None,
        container: None,
    };
    let plan = plan::plan_compress(&info, &caps(&[]), &opts).unwrap();
    let mut r = Report::new("compress", &plan);
    r.output = PathBuf::from("/v/a.compressed.mp4");
    r.set_sizes(10_000_000, 5_800_000);
    r.elapsed_secs = 65.0;
    r.vmaf_mean = Some(90.12);
    r.vmaf_p5 = Some(86.4);
    r.vmaf_target = Some(93.0);
    r.warnings
        .push(nago_core::ops::vmaf_warning(90.12, Quality::High).unwrap());
    r
}

#[tokio::test]
async fn tela_com_job_em_execucao_concluido_e_com_falha() {
    let tmp = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, tmp.path());
    let report = compress_report(&input).await;
    let mut h = harness();
    enqueue(
        &mut h,
        compress(Path::new("/v/a.mp4"), Quality::High),
        "/v/a.compressed.mp4",
    );
    enqueue(
        &mut h,
        compress(Path::new("/v/b.mp4"), Quality::High),
        "/v/b.compressed.mp4",
    );
    enqueue(&mut h, cut(Path::new("/v/c.mp4")), "/v/c.cut.mp4");
    h.send(Msg::JobStarted {
        id: 1,
        output: "/v/a.compressed.mp4".into(),
    });
    h.send(Msg::Job {
        id: 1,
        event: JobEvent::Phase {
            name: "codificando".into(),
        },
    });
    h.send(Msg::Job {
        id: 1,
        event: JobEvent::Progress(Progress {
            ratio: 0.45,
            speed: Some(2.3),
            eta_secs: Some(12.0),
            out_time_secs: 4.5,
        }),
    });
    h.key(KeyCode::Tab);
    insta::assert_snapshot!("fila_em_execucao", h.screen());

    h.send(Msg::JobFinished {
        id: 1,
        result: Ok(report),
    });
    assert!(matches!(h.app.queue.jobs[0].state, JobState::Done(_)));
    let screen = h.screen();
    assert!(
        screen.contains("VMAF: média 90.12 · p5 86.40 (meta 93 ✗)"),
        "{screen}"
    );
    assert!(screen.contains("experimente --quality visually-lossless"));
    insta::assert_snapshot!("fila_concluido_com_aviso", screen);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn falha_do_ffmpeg_mostra_erro_e_log_e_a_fila_segue() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let input = fixture_in(Fixture::H264Aac360p, tmp.path());
    let real = Tools::locate().await.unwrap();
    let fake = tmp.path().join("ffmpeg-quebrado");
    std::fs::write(
        &fake,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n*-progress*) echo 'Error while decoding stream #0:0: Invalid data found' >&2; \
             echo 'Conversion failed!' >&2; exit 1;;\n*) exec {} \"$@\";;\nesac\n",
            real.ffmpeg.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut tools = real.clone();
    tools.ffmpeg = fake;
    let mut d = Driver::new(tmp.path(), tools).await;
    d.enqueue(vec![convert(&input, "mkv"), cut(&input)]).await;
    d.until(all_finished).await;
    let jobs = &d.app().queue.jobs;
    let JobState::Failed { message, log_tail } = &jobs[0].state else {
        panic!("{:?}", jobs[0].state);
    };
    assert!(!message.is_empty());
    assert!(
        log_tail.iter().any(|l| l.contains("Conversion failed!")),
        "{log_tail:?}"
    );
    // o segundo job também usa o ffmpeg quebrado, mas foi tentado: a fila seguiu
    assert!(matches!(jobs[1].state, JobState::Failed { .. }));
    d.h.key(KeyCode::Tab);
    let screen =
        d.h.screen()
            .replace(&tmp.path().display().to_string(), "/tmp/teste");
    assert!(screen.contains("últimas linhas do ffmpeg"), "{screen}");
    assert!(screen.contains("Conversion failed!"));
    // nenhum arquivo parcial ficou para trás
    assert_eq!(files(tmp.path()), ["ffmpeg-quebrado", "h264_aac_360p.mp4"]);
    insta::assert_snapshot!(
        "fila_com_falha",
        screen
            .lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ") + "\n")
            .collect::<String>()
    );
}
