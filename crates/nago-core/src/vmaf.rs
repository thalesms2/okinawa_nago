//! Medição de qualidade com libvmaf (spec `video-compression` › Verificação, design D10).

use std::ffi::OsString;
use std::path::Path;

use serde_json::Value;

use crate::runner::Runner;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VmafScore {
    pub mean: f64,
    /// Percentil 5 dos frames (os piores momentos).
    pub p5: f64,
}

/// Argumentos para medir `distorted` contra `reference` (mesma resolução).
pub fn vmaf_args(
    distorted: &Path,
    reference: &Path,
    log_path: &Path,
    threads: usize,
) -> Vec<OsString> {
    let graph = format!(
        "[0:v]format=yuv420p10le,setpts=PTS-STARTPTS[d];\
         [1:v]format=yuv420p10le,setpts=PTS-STARTPTS[r];\
         [d][r]libvmaf=model=version=vmaf_v0.6.1:n_threads={threads}:log_fmt=json:log_path={}",
        escape_filter_value(&log_path.to_string_lossy())
    );
    let mut args: Vec<OsString> =
        vec!["-i".into(), distorted.into(), "-i".into(), reference.into()];
    for a in ["-lavfi", &graph, "-f", "null", "-"] {
        args.push(a.into());
    }
    args
}

/// Escapa `\`, `:` e `'` para uso como valor de opção num filtergraph.
fn escape_filter_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | ':' | '\'' | ',' | ';' | '[' | ']') {
            out.push_str("\\\\");
        }
        out.push(c);
    }
    out
}

/// Lê média e percentil 5 do log JSON da libvmaf.
pub fn parse_log(json: &Value) -> Result<VmafScore> {
    let mut scores: Vec<f64> = json
        .get("frames")
        .and_then(Value::as_array)
        .map(|frames| {
            frames
                .iter()
                .filter_map(|f| f.get("metrics")?.get("vmaf")?.as_f64())
                .collect()
        })
        .unwrap_or_default();
    if scores.is_empty() {
        return Err(Error::Other("o log do VMAF não tem frames".into()));
    }
    scores.sort_by(f64::total_cmp);
    let mean = json
        .pointer("/pooled_metrics/vmaf/mean")
        .and_then(Value::as_f64)
        .unwrap_or_else(|| scores.iter().sum::<f64>() / scores.len() as f64);
    let p5 = scores[((scores.len() - 1) as f64 * 0.05).floor() as usize];
    Ok(VmafScore { mean, p5 })
}

/// Executa a medição (fase "medindo VMAF").
pub async fn measure(
    runner: &Runner,
    distorted: &Path,
    reference: &Path,
    duration: f64,
) -> Result<VmafScore> {
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("vmaf.json");
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let args = vmaf_args(distorted, reference, &log, threads);
    runner.run_ffmpeg("medindo VMAF", &args, duration).await?;
    let text = std::fs::read_to_string(&log)
        .map_err(|e| Error::Other(format!("não foi possível ler o log do VMAF: {e}")))?;
    let json: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Other(format!("log do VMAF inválido: {e}")))?;
    parse_log(&json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn media_e_percentil_5() {
        let frames: Vec<Value> = (0..100)
            .map(|i| json!({"frameNum": i, "metrics": {"vmaf": 90.0 + (i as f64) / 10.0}}))
            .collect();
        let log = json!({"frames": frames, "pooled_metrics": {"vmaf": {"mean": 94.95}}});
        let s = parse_log(&log).unwrap();
        assert_eq!(s.mean, 94.95);
        assert!((s.p5 - 90.4).abs() < 1e-9, "{}", s.p5);
    }

    #[test]
    fn log_vazio_e_erro() {
        assert!(parse_log(&json!({"frames": []})).is_err());
    }

    #[test]
    fn escape_de_caminho() {
        assert_eq!(
            escape_filter_value("/tmp/a b/vmaf.json"),
            "/tmp/a b/vmaf.json"
        );
        assert_eq!(escape_filter_value("C:/x"), "C\\\\:/x");
    }
}
