//! Operações de alto nível: probe → plano → execução → relatório.
//!
//! Cada operação tem duas etapas: `prepare_*` (probe, plano e resolução da saída, sem
//! executar nada) e [`execute`]. A CLI usa [`compress`]/[`convert`]/[`cut`], que fazem
//! as duas; a TUI prepara ao editar o formulário e executa quando chega a vez do job.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::caps::Capabilities;
use crate::container::Container;
use crate::encoders::Codec;
use crate::output::{self, DefaultName, Reservations};
use crate::plan::{self, CompressOptions, ConvertOptions, CutOptions, Plan};
use crate::probe::{self, MediaInfo};
use crate::profiles::Quality;
use crate::report::Report;
use crate::runner::{Runner, TempOutput};
use crate::timefmt;
use crate::tools::Tools;
use crate::vmaf;
use crate::{Error, Result};

/// Opções comuns a todas as operações que gravam um arquivo.
#[derive(Debug, Clone, Default)]
pub struct OutputOptions {
    /// `-o` (arquivo ou diretório).
    pub path: Option<PathBuf>,
    pub overwrite: bool,
}

impl OutputOptions {
    /// Container indicado pela extensão de `-o` (`None` sem `-o`, diretório ou sem extensão).
    fn container(&self) -> Result<Option<Container>> {
        let Some(path) = &self.path else {
            return Ok(None);
        };
        if path.is_dir() {
            return Ok(None);
        }
        match path.extension() {
            None => Ok(None),
            Some(ext) => {
                let ext = ext.to_string_lossy();
                Container::parse(&ext).map(Some).ok_or_else(|| {
                    Error::usage(format!(
                        "extensão `.{ext}` não suportada em -o; formatos suportados: {}",
                        Container::names()
                    ))
                })
            }
        }
    }
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Lê o arquivo, a menos que o chamador já tenha o `MediaInfo` dele.
async fn probe_or(tools: &Tools, input: &Path, known: Option<&MediaInfo>) -> Result<MediaInfo> {
    match known {
        Some(info) if info.path == input => Ok(info.clone()),
        _ => probe::probe(tools, input).await,
    }
}

/// O que é específico de cada operação num [`Prepared`].
#[derive(Debug, Clone)]
pub enum PreparedKind {
    Compress {
        quality: Quality,
        verify: bool,
        force: bool,
    },
    Convert,
    Cut {
        exact: bool,
        requested_start: f64,
        /// Início real da saída (keyframe no corte rápido).
        actual_start: f64,
        end: f64,
    },
}

/// Uma operação validada e pronta para executar.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub kind: PreparedKind,
    pub info: MediaInfo,
    pub plan: Plan,
    /// Caminho final da saída (já resolvido, livre no momento da preparação).
    pub output: PathBuf,
    pub phase: &'static str,
}

impl Prepared {
    pub fn input(&self) -> &Path {
        &self.plan.input
    }

    /// Encoder de vídeo que será usado (`None` quando o vídeo é copiado).
    pub fn encoder(&self) -> Option<&str> {
        self.plan.video_encoder.as_deref()
    }

    pub fn warnings(&self) -> &[String] {
        &self.plan.warnings
    }

    pub fn operation(&self) -> &'static str {
        match self.kind {
            PreparedKind::Compress { .. } => "compress",
            PreparedKind::Convert => "convert",
            PreparedKind::Cut { .. } => "cut",
        }
    }
}

/// Executa uma operação preparada: grava num temporário, aplica as verificações e
/// persiste a saída.
pub async fn execute(runner: &Runner, prepared: &Prepared) -> Result<Report> {
    let started = Instant::now();
    let plan = &prepared.plan;
    let input = prepared.input();
    for w in &plan.warnings {
        runner.warn(w.clone());
    }
    let temp = TempOutput::new(&prepared.output)?;
    let args = plan::build_args(plan, temp.path());
    runner
        .run_ffmpeg(prepared.phase, &args, plan.expected_duration)
        .await?;

    let input_size = file_size(input);
    let output_size = temp.size();
    if let PreparedKind::Compress { force: false, .. } = prepared.kind
        && output_size >= input_size
    {
        drop(temp);
        return Err(Error::NotSmaller {
            input_size,
            output_size,
        });
    }
    let output = temp.persist_like(input)?;

    let mut report = Report::new(prepared.operation(), plan);
    match prepared.kind {
        PreparedKind::Compress {
            quality,
            verify: true,
            ..
        } => {
            let info = &prepared.info;
            let score = vmaf::measure(runner, &output, input, info.duration_secs).await?;
            report.vmaf_mean = Some(score.mean);
            report.vmaf_p5 = Some(score.p5);
            report.vmaf_target = Some(quality.vmaf_target());
            if let Some(msg) = vmaf_warning(score.mean, quality) {
                runner.warn(msg.clone());
                report.warnings.push(msg);
            }
            if info.primary_video().is_some_and(|v| v.is_hdr()) {
                let msg = "o modelo VMAF padrão é SDR: para vídeo HDR o valor é apenas indicativo"
                    .to_owned();
                runner.warn(msg.clone());
                report.warnings.push(msg);
            }
        }
        PreparedKind::Cut {
            requested_start,
            actual_start,
            end,
            ..
        } => {
            report.requested_start_secs = Some(requested_start);
            report.actual_start_secs = Some(actual_start);
            report.duration_secs = Some(end - actual_start);
        }
        _ => {}
    }
    report.output = output;
    report.set_sizes(input_size, output_size);
    report.elapsed_secs = started.elapsed().as_secs_f64();
    Ok(report)
}

/// Executa contando no tempo do relatório também a preparação (iniciada em `started`).
async fn execute_since(runner: &Runner, prepared: &Prepared, started: Instant) -> Result<Report> {
    let mut report = execute(runner, prepared).await?;
    report.elapsed_secs = started.elapsed().as_secs_f64();
    Ok(report)
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CompressRequest {
    pub input: PathBuf,
    pub output: OutputOptions,
    pub quality: Quality,
    pub codec: Codec,
    pub gpu: bool,
    pub audio_bitrate: Option<u32>,
    pub verify: bool,
    pub force: bool,
}

pub async fn prepare_compress(
    tools: &Tools,
    caps: &Capabilities,
    req: &CompressRequest,
    known: Option<&MediaInfo>,
    reserved: &Reservations,
) -> Result<Prepared> {
    let info = probe_or(tools, &req.input, known).await?;
    if req.verify && !caps.vmaf {
        return Err(Error::Unsupported(
            "--verify precisa de um ffmpeg compilado com libvmaf, que não está disponível neste ffmpeg".into(),
        ));
    }
    let opts = CompressOptions {
        quality: req.quality,
        codec: req.codec,
        gpu: req.gpu,
        audio_bitrate: req.audio_bitrate,
        container: req.output.container()?,
    };
    let plan = plan::plan_compress(&info, caps, &opts)?;
    let output = output::resolve(
        &req.input,
        req.output.path.as_deref(),
        DefaultName {
            suffix: Some("compressed"),
            ext: &plan.ext,
        },
        req.output.overwrite,
        reserved,
    )?;
    Ok(Prepared {
        kind: PreparedKind::Compress {
            quality: req.quality,
            verify: req.verify,
            force: req.force,
        },
        info,
        plan,
        output,
        phase: "codificando",
    })
}

pub async fn compress(
    runner: &Runner,
    caps: &Capabilities,
    req: &CompressRequest,
) -> Result<Report> {
    let started = Instant::now();
    let prepared = prepare_compress(runner.tools(), caps, req, None, &Reservations::new()).await?;
    execute_since(runner, &prepared, started).await
}

/// Aviso quando o VMAF médio fica abaixo da meta do perfil.
pub fn vmaf_warning(mean: f64, quality: Quality) -> Option<String> {
    let target = quality.vmaf_target();
    if mean >= target {
        return None;
    }
    let mut msg = format!(
        "VMAF médio {mean:.2} ficou abaixo da meta {target:.0} do perfil {}",
        quality.name()
    );
    if quality != Quality::VisuallyLossless {
        msg.push_str("; experimente --quality visually-lossless");
    }
    Some(msg)
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ConvertRequest {
    pub input: PathBuf,
    pub output: OutputOptions,
    /// `--to`; se ausente, vem da extensão de `-o`.
    pub format: Option<String>,
    pub audio_track: Option<usize>,
    pub compat: bool,
}

pub async fn prepare_convert(
    tools: &Tools,
    caps: &Capabilities,
    req: &ConvertRequest,
    known: Option<&MediaInfo>,
    reserved: &Reservations,
) -> Result<Prepared> {
    let target = match &req.format {
        Some(f) => Container::parse(f).ok_or_else(|| {
            Error::usage(format!(
                "formato `{f}` não suportado; formatos suportados: {}",
                Container::names()
            ))
        })?,
        None => req.output.container()?.ok_or_else(|| {
            Error::usage(format!(
                "informe o formato com --to ou pela extensão de -o ({})",
                Container::names()
            ))
        })?,
    };
    let info = probe_or(tools, &req.input, known).await?;
    let opts = ConvertOptions {
        target,
        audio_track: req.audio_track,
        compat: req.compat,
    };
    let plan = plan::plan_convert(&info, caps, &opts)?;
    let output = output::resolve(
        &req.input,
        req.output.path.as_deref(),
        DefaultName {
            suffix: None,
            ext: target.ext(),
        },
        req.output.overwrite,
        reserved,
    )?;
    let phase = if plan.transcodes_anything() {
        "convertendo"
    } else {
        "remuxando"
    };
    Ok(Prepared {
        kind: PreparedKind::Convert,
        info,
        plan,
        output,
        phase,
    })
}

pub async fn convert(runner: &Runner, caps: &Capabilities, req: &ConvertRequest) -> Result<Report> {
    let started = Instant::now();
    let prepared = prepare_convert(runner.tools(), caps, req, None, &Reservations::new()).await?;
    execute_since(runner, &prepared, started).await
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct CutRequest {
    pub input: PathBuf,
    pub output: OutputOptions,
    pub from: Option<f64>,
    pub to: Option<f64>,
    pub duration: Option<f64>,
    pub exact: bool,
}

pub async fn prepare_cut(
    tools: &Tools,
    caps: &Capabilities,
    req: &CutRequest,
    known: Option<&MediaInfo>,
    reserved: &Reservations,
) -> Result<Prepared> {
    let container = req.output.container()?;
    let info = probe_or(tools, &req.input, known).await?;
    let (range, end_warning) =
        plan::resolve_cut_range(info.duration_secs, req.from, req.to, req.duration)?;

    let mut extra_warnings: Vec<String> = end_warning.into_iter().collect();
    let keyframe_start = {
        let kf = probe::keyframe_at_or_before(tools, &info, range.start).await?;
        if let Some(kf) = kf
            && !req.exact
            && range.start - kf > 0.001
        {
            extra_warnings.push(format!(
                "o corte rápido começa no keyframe anterior: início real {} (pedido {}); use --exact para corte preciso ao frame",
                timefmt::format(kf),
                timefmt::format(range.start)
            ));
        }
        kf
    };

    let opts = CutOptions {
        range,
        exact: req.exact,
        keyframe_start,
        container,
    };
    let mut plan = plan::plan_cut(&info, caps, &opts)?;
    plan.warnings.splice(0..0, extra_warnings);
    let output = output::resolve(
        &req.input,
        req.output.path.as_deref(),
        DefaultName {
            suffix: Some("cut"),
            ext: &plan.ext,
        },
        req.output.overwrite,
        reserved,
    )?;
    let actual_start = if req.exact {
        range.start
    } else {
        keyframe_start.unwrap_or(range.start)
    };
    Ok(Prepared {
        kind: PreparedKind::Cut {
            exact: req.exact,
            requested_start: range.start,
            actual_start,
            end: range.end,
        },
        info,
        plan,
        output,
        phase: if req.exact {
            "cortando (exato)"
        } else {
            "cortando"
        },
    })
}

pub async fn cut(runner: &Runner, caps: &Capabilities, req: &CutRequest) -> Result<Report> {
    let started = Instant::now();
    let prepared = prepare_cut(runner.tools(), caps, req, None, &Reservations::new()).await?;
    execute_since(runner, &prepared, started).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aviso_de_vmaf_abaixo_da_meta() {
        let msg = vmaf_warning(90.0, Quality::High).unwrap();
        assert!(msg.contains("90.00") && msg.contains("meta 93"), "{msg}");
        assert!(msg.contains("--quality visually-lossless"));
        assert_eq!(vmaf_warning(93.0, Quality::High), None);
        assert_eq!(vmaf_warning(96.0, Quality::VisuallyLossless), None);
        let vl = vmaf_warning(94.0, Quality::VisuallyLossless).unwrap();
        assert!(!vl.contains("experimente"));
    }
}
