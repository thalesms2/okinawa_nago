//! Operações de alto nível: probe → plano → execução → relatório.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::caps::Capabilities;
use crate::container::Container;
use crate::encoders::Codec;
use crate::output::{self, DefaultName};
use crate::plan::{self, CompressOptions, ConvertOptions, CutOptions, Plan};
use crate::probe;
use crate::profiles::Quality;
use crate::report::Report;
use crate::runner::{Runner, TempOutput};
use crate::timefmt;
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

/// Executa o plano gravando num temporário; devolve o temporário pronto para persistir.
async fn execute(runner: &Runner, plan: &Plan, output: &Path, phase: &str) -> Result<TempOutput> {
    for w in &plan.warnings {
        runner.warn(w.clone());
    }
    let temp = TempOutput::new(output)?;
    let args = plan::build_args(plan, temp.path());
    runner
        .run_ffmpeg(phase, &args, plan.expected_duration)
        .await?;
    Ok(temp)
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

pub async fn compress(
    runner: &Runner,
    caps: &Capabilities,
    req: &CompressRequest,
) -> Result<Report> {
    let started = Instant::now();
    let info = probe::probe(runner.tools(), &req.input).await?;
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
    )?;

    let temp = execute(runner, &plan, &output, "codificando").await?;
    let input_size = file_size(&req.input);
    let output_size = temp.size();
    if output_size >= input_size && !req.force {
        drop(temp);
        return Err(Error::NotSmaller {
            input_size,
            output_size,
        });
    }
    let output = temp.persist_like(&req.input)?;

    let mut report = Report::new("compress", &plan);
    if req.verify {
        let score = vmaf::measure(runner, &output, &req.input, info.duration_secs).await?;
        let target = req.quality.vmaf_target();
        report.vmaf_mean = Some(score.mean);
        report.vmaf_p5 = Some(score.p5);
        report.vmaf_target = Some(target);
        if let Some(msg) = vmaf_warning(score.mean, req.quality) {
            runner.warn(msg.clone());
            report.warnings.push(msg);
        }
        if info.primary_video().is_some_and(|v| v.is_hdr()) {
            let msg =
                "o modelo VMAF padrão é SDR: para vídeo HDR o valor é apenas indicativo".to_owned();
            runner.warn(msg.clone());
            report.warnings.push(msg);
        }
    }
    report.output = output;
    report.set_sizes(input_size, output_size);
    report.elapsed_secs = started.elapsed().as_secs_f64();
    Ok(report)
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

pub async fn convert(runner: &Runner, caps: &Capabilities, req: &ConvertRequest) -> Result<Report> {
    let started = Instant::now();
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
    let info = probe::probe(runner.tools(), &req.input).await?;
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
    )?;
    let phase = if plan.transcodes_anything() {
        "convertendo"
    } else {
        "remuxando"
    };
    let temp = execute(runner, &plan, &output, phase).await?;
    let output_size = temp.size();
    let output = temp.persist_like(&req.input)?;

    let mut report = Report::new("convert", &plan);
    report.output = output;
    report.set_sizes(file_size(&req.input), output_size);
    report.elapsed_secs = started.elapsed().as_secs_f64();
    Ok(report)
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

pub async fn cut(runner: &Runner, caps: &Capabilities, req: &CutRequest) -> Result<Report> {
    let started = Instant::now();
    let container = req.output.container()?;
    let info = probe::probe(runner.tools(), &req.input).await?;
    let (range, end_warning) =
        plan::resolve_cut_range(info.duration_secs, req.from, req.to, req.duration)?;

    let mut extra_warnings: Vec<String> = end_warning.into_iter().collect();
    let keyframe_start = {
        let kf = probe::keyframe_at_or_before(runner.tools(), &info, range.start).await?;
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
    )?;
    let phase = if req.exact {
        "cortando (exato)"
    } else {
        "cortando"
    };
    let temp = execute(runner, &plan, &output, phase).await?;
    let output_size = temp.size();
    let output = temp.persist_like(&req.input)?;

    let actual_start = if req.exact {
        range.start
    } else {
        keyframe_start.unwrap_or(range.start)
    };
    let mut report = Report::new("cut", &plan);
    report.output = output;
    report.set_sizes(file_size(&req.input), output_size);
    report.requested_start_secs = Some(range.start);
    report.actual_start_secs = Some(actual_start);
    report.duration_secs = Some(range.end - actual_start);
    report.elapsed_secs = started.elapsed().as_secs_f64();
    Ok(report)
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
