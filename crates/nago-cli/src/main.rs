mod args;
mod progress;
mod render;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use nago_core::caps;
use nago_core::encoders::Codec;
use nago_core::ops::{self, CompressRequest, ConvertRequest, CutRequest, OutputOptions};
use nago_core::profiles::Quality;
use nago_core::report::Report;
use nago_core::runner::Runner;
use nago_core::timefmt;
use nago_core::tools::Tools;
use tokio_util::sync::CancellationToken;

use args::{
    Cli, CodecArg, Command, CompressArgs, ConvertArgs, CutArgs, GlobalOpts, InfoArgs, QualityArg,
};

fn main() -> ExitCode {
    // clap já sai com código 2 em erro de uso e 0 em --help/--version.
    let cli = Cli::parse();
    init_tracing(cli.global.verbose);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("erro: não foi possível iniciar o runtime: {e}");
            return ExitCode::from(1);
        }
    };

    let result = runtime.block_on(run(cli));
    // Garante que tarefas pendentes (ex.: limpeza de temporários) terminem.
    runtime.shutdown_timeout(std::time::Duration::from_secs(1));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            print_error(&err);
            ExitCode::from(exit_code(&err))
        }
    }
}

fn init_tracing(verbose: bool) {
    let filter = if verbose {
        "nago_core=debug,nago=debug"
    } else {
        "warn"
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("NAGO_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_writer(std::io::stderr)
        .without_time()
        .try_init();
}

/// Converte um erro em código de saída: erros do core sabem o seu código,
/// qualquer outro é erro de execução (1).
fn exit_code(err: &anyhow::Error) -> u8 {
    err.downcast_ref::<nago_core::Error>()
        .map(nago_core::Error::exit_code)
        .unwrap_or(1)
}

fn print_error(err: &anyhow::Error) {
    eprintln!("erro: {err:#}");
    if let Some(nago_core::Error::FfmpegFailed {
        log_tail, log_file, ..
    }) = err.downcast_ref::<nago_core::Error>()
    {
        if !log_tail.is_empty() {
            eprintln!("últimas linhas do ffmpeg:");
            for line in log_tail {
                eprintln!("  {line}");
            }
        }
        match log_file {
            Some(p) => eprintln!("log completo: {}", p.display()),
            None => eprintln!("rode com --verbose para salvar o log completo do ffmpeg"),
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    // Todo comando depende do ffmpeg: valida antes de qualquer operação de mídia.
    let tools = Tools::locate().await?;
    let g = cli.global.clone();
    match cli.command {
        Command::Info(args) => info(&tools, &g, args).await,
        Command::Doctor => doctor(&tools, &g).await,
        Command::Cut(args) => {
            let req = cut_request(args, &g)?;
            run_job(tools, &g, |runner, caps| async move {
                ops::cut(&runner, &caps, &req).await
            })
            .await
        }
        Command::Convert(args) => {
            let req = convert_request(args, &g);
            run_job(tools, &g, |runner, caps| async move {
                ops::convert(&runner, &caps, &req).await
            })
            .await
        }
        Command::Compress(args) => {
            let req = compress_request(args, &g);
            run_job(tools, &g, |runner, caps| async move {
                ops::compress(&runner, &caps, &req).await
            })
            .await
        }
    }
}

fn output_opts(path: Option<PathBuf>, g: &GlobalOpts) -> OutputOptions {
    OutputOptions {
        path,
        overwrite: g.overwrite,
    }
}

fn cut_request(a: CutArgs, g: &GlobalOpts) -> anyhow::Result<CutRequest> {
    let parse = |v: &Option<String>| v.as_deref().map(timefmt::parse).transpose();
    Ok(CutRequest {
        from: parse(&a.from)?,
        to: parse(&a.to)?,
        duration: parse(&a.duration)?,
        exact: a.exact,
        output: output_opts(a.output, g),
        input: a.input,
    })
}

fn convert_request(a: ConvertArgs, g: &GlobalOpts) -> ConvertRequest {
    ConvertRequest {
        input: a.input,
        output: output_opts(a.output, g),
        format: a.format,
        audio_track: a.audio_track,
        compat: a.compat,
    }
}

fn compress_request(a: CompressArgs, g: &GlobalOpts) -> CompressRequest {
    CompressRequest {
        input: a.input,
        output: output_opts(a.output, g),
        quality: match a.quality {
            QualityArg::VisuallyLossless => Quality::VisuallyLossless,
            QualityArg::High => Quality::High,
            QualityArg::Balanced => Quality::Balanced,
            QualityArg::Small => Quality::Small,
        },
        codec: match a.codec {
            CodecArg::Av1 => Codec::Av1,
            CodecArg::Hevc => Codec::Hevc,
            CodecArg::H264 => Codec::H264,
        },
        gpu: a.gpu,
        audio_bitrate: a.audio_bitrate,
        verify: a.verify,
        force: a.force,
    }
}

/// Monta o runner (eventos, cancelamento por sinal, log) e executa a operação.
async fn run_job<F, Fut>(tools: Tools, g: &GlobalOpts, op: F) -> anyhow::Result<()>
where
    F: FnOnce(Runner, caps::Capabilities) -> Fut,
    Fut: std::future::Future<Output = nago_core::Result<Report>>,
{
    let caps = caps::load_or_detect(&tools).await?;
    let cancel = CancellationToken::new();
    spawn_signal_handler(cancel.clone());

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let renderer = tokio::spawn(progress::render(rx));
    let log_dir = g
        .verbose
        .then(|| dirs_log_dir().unwrap_or_else(std::env::temp_dir));
    let runner = Runner::new(tools)
        .with_events(tx)
        .with_cancel(cancel)
        .with_log_dir(log_dir);

    let result = op(runner, caps).await;
    // O runner (e o transmissor de eventos) já foi consumido: o renderizador termina.
    let _ = renderer.await;
    let report = result?;

    if g.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render::report(&report));
    }
    Ok(())
}

fn dirs_log_dir() -> Option<PathBuf> {
    std::env::var_os("NAGO_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(|d| PathBuf::from(d).join("nago")))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/nago")))
        .map(|d| d.join("logs"))
}

fn spawn_signal_handler(cancel: CancellationToken) {
    tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut term = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                    cancel.cancel();
                    return;
                }
            };
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
        }
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
        cancel.cancel();
    });
}

async fn info(tools: &Tools, g: &GlobalOpts, args: InfoArgs) -> anyhow::Result<()> {
    let info = nago_core::probe::probe(tools, &args.input).await?;
    if g.json {
        println!("{}", serde_json::to_string_pretty(&info)?);
    } else {
        print!("{}", render::media_info(&info));
    }
    Ok(())
}

async fn doctor(tools: &Tools, g: &GlobalOpts) -> anyhow::Result<()> {
    use nago_core::caps::EncoderStatus;
    use nago_core::encoders::VIDEO_ENCODERS;

    let caps = caps::refresh(tools).await?;
    if g.json {
        let encoders: Vec<_> = VIDEO_ENCODERS
            .iter()
            .map(|e| {
                serde_json::json!({
                    "name": e.name,
                    "codec": e.codec,
                    "backend": e.backend,
                    "status": caps.status(e),
                })
            })
            .collect();
        let report = serde_json::json!({
            "ffmpeg": tools.ffmpeg,
            "ffprobe": tools.ffprobe,
            "version": tools.version.to_string(),
            "libvmaf": caps.vmaf,
            "encoders": encoders,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    println!("ffmpeg   {} ({})", tools.version, tools.ffmpeg.display());
    println!("ffprobe  {}", tools.ffprobe.display());
    println!(
        "libvmaf  {}",
        if caps.vmaf {
            "disponível (--verify funciona)"
        } else {
            "ausente (--verify indisponível)"
        }
    );
    for codec in Codec::ALL {
        println!();
        println!("{}", codec.label());
        for e in VIDEO_ENCODERS.iter().filter(|e| e.codec == codec) {
            let status = match caps.status(e) {
                EncoderStatus::Available => "✓ funcional".to_owned(),
                EncoderStatus::Unavailable(reason) => format!("✗ indisponível: {reason}"),
                EncoderStatus::NotCompiled => "– não compilado neste ffmpeg".to_owned(),
            };
            println!("  {:<11} {:<9} {status}", e.name, e.backend.label());
        }
    }
    Ok(())
}
