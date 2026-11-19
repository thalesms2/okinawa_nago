//! Detecção de capacidades do FFmpeg/hardware e escolha do encoder
//! (spec `encoder-selection`, design D7).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::encoders::{Backend, Codec, Encoder, VIDEO_ENCODERS, vaapi_device};
use crate::tools::Tools;
use crate::{Error, Result};

/// O que o ffmpeg instalado consegue fazer nesta máquina.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Capabilities {
    pub key: CacheKey,
    /// Todos os encoders compilados (vídeo, áudio, legenda).
    pub compiled: BTreeSet<String>,
    pub vmaf: bool,
    /// Resultado do teste real de cada encoder de hardware compilado.
    pub hardware: Vec<HwStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HwStatus {
    pub encoder: String,
    pub working: bool,
    /// Motivo da falha, quando `working == false`.
    pub reason: Option<String>,
}

/// Identifica o binário do ffmpeg para invalidar o cache quando ele muda.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheKey {
    pub ffmpeg: PathBuf,
    pub mtime_secs: u64,
    pub version: String,
}

/// Situação de um encoder para exibição (`nago doctor`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status", content = "reason")]
pub enum EncoderStatus {
    Available,
    Unavailable(String),
    NotCompiled,
}

impl CacheKey {
    pub fn for_tools(tools: &Tools) -> CacheKey {
        let ffmpeg = tools
            .ffmpeg
            .canonicalize()
            .unwrap_or_else(|_| tools.ffmpeg.clone());
        let mtime_secs = std::fs::metadata(&ffmpeg)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        CacheKey {
            ffmpeg,
            mtime_secs,
            version: tools.version.to_string(),
        }
    }
}

impl Capabilities {
    pub fn is_compiled(&self, name: &str) -> bool {
        self.compiled.contains(name)
    }

    pub fn status(&self, encoder: &Encoder) -> EncoderStatus {
        if !self.is_compiled(encoder.name) {
            return EncoderStatus::NotCompiled;
        }
        if encoder.backend == Backend::Software {
            return EncoderStatus::Available;
        }
        match self.hardware.iter().find(|h| h.encoder == encoder.name) {
            Some(h) if h.working => EncoderStatus::Available,
            Some(h) => EncoderStatus::Unavailable(
                h.reason.clone().unwrap_or_else(|| "falhou no teste".into()),
            ),
            None => EncoderStatus::Unavailable("não testado".into()),
        }
    }

    pub fn is_usable(&self, encoder: &Encoder) -> bool {
        self.status(encoder) == EncoderStatus::Available
    }
}

// ---------------------------------------------------------------------------
// Detecção

/// Carrega do cache ou detecta (e grava o cache).
pub async fn load_or_detect(tools: &Tools) -> Result<Capabilities> {
    let key = CacheKey::for_tools(tools);
    if let Some(dir) = cache_dir() {
        if let Some(caps) = load_cached(&dir, &key) {
            return Ok(caps);
        }
    }
    let caps = detect(tools).await?;
    if let Some(dir) = cache_dir() {
        if let Err(e) = save_cache(&dir, &caps) {
            tracing::debug!("não foi possível gravar o cache de capacidades: {e}");
        }
    }
    Ok(caps)
}

/// Sempre refaz a detecção (usado por `nago doctor`) e atualiza o cache.
pub async fn refresh(tools: &Tools) -> Result<Capabilities> {
    let caps = detect(tools).await?;
    if let Some(dir) = cache_dir() {
        let _ = save_cache(&dir, &caps);
    }
    Ok(caps)
}

pub async fn detect(tools: &Tools) -> Result<Capabilities> {
    let encoders_out = ffmpeg_output(&tools.ffmpeg, &["-hide_banner", "-encoders"]).await?;
    let filters_out = ffmpeg_output(&tools.ffmpeg, &["-hide_banner", "-filters"]).await?;
    let compiled = parse_encoders(&encoders_out);
    let vmaf = has_filter(&filters_out, "libvmaf");

    let candidates: Vec<Encoder> = VIDEO_ENCODERS
        .into_iter()
        .filter(|e| e.backend.is_hardware() && compiled.contains(e.name))
        .collect();
    let tests = candidates
        .iter()
        .map(|e| test_hw_encoder(tools.ffmpeg.clone(), *e));
    let hardware = futures_join_all(tests).await;

    Ok(Capabilities {
        key: CacheKey::for_tools(tools),
        compiled,
        vmaf,
        hardware,
    })
}

async fn futures_join_all<F: std::future::Future<Output = HwStatus> + Send + 'static>(
    futs: impl Iterator<Item = F>,
) -> Vec<HwStatus> {
    let handles: Vec<_> = futs.map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        if let Ok(status) = h.await {
            out.push(status);
        }
    }
    out
}

async fn ffmpeg_output(ffmpeg: &Path, args: &[&str]) -> Result<String> {
    let out = tokio::process::Command::new(ffmpeg)
        .args(args)
        .output()
        .await?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Faz parse de `ffmpeg -encoders`: devolve os nomes dos encoders.
pub fn parse_encoders(text: &str) -> BTreeSet<String> {
    text.lines()
        .skip_while(|l| !l.trim_start().starts_with("------"))
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let flags = it.next()?;
            let name = it.next()?;
            matches!(flags.chars().next(), Some('V' | 'A' | 'S')).then(|| name.to_owned())
        })
        .collect()
}

/// Verifica se `ffmpeg -filters` lista o filtro.
pub fn has_filter(text: &str, name: &str) -> bool {
    text.lines()
        .any(|l| l.split_whitespace().nth(1) == Some(name))
}

/// Argumentos do teste funcional de ~0,1 s de um encoder de hardware.
pub fn hw_test_args(encoder: &Encoder) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error"]
        .map(String::from)
        .to_vec();
    if encoder.backend == Backend::Vaapi {
        args.extend(["-vaapi_device".into(), vaapi_device()]);
    }
    args.extend(["-f", "lavfi", "-i", "color=c=gray:s=256x256:r=10:d=0.1"].map(String::from));
    match encoder.backend {
        Backend::Vaapi => args.extend(["-vf".into(), "format=nv12,hwupload".into()]),
        Backend::Qsv => args.extend(["-pix_fmt".into(), "nv12".into()]),
        _ => {}
    }
    args.extend([
        "-c:v".into(),
        encoder.name.into(),
        "-f".into(),
        "null".into(),
        "-".into(),
    ]);
    args
}

async fn test_hw_encoder(ffmpeg: PathBuf, encoder: Encoder) -> HwStatus {
    let run = tokio::process::Command::new(&ffmpeg)
        .args(hw_test_args(&encoder))
        .kill_on_drop(true)
        .output();
    let (working, reason) = match tokio::time::timeout(Duration::from_secs(20), run).await {
        Ok(Ok(out)) if out.status.success() => (true, None),
        Ok(Ok(out)) => (
            false,
            Some(failure_reason(&String::from_utf8_lossy(&out.stderr))),
        ),
        Ok(Err(e)) => (false, Some(format!("falha ao executar o ffmpeg: {e}"))),
        Err(_) => (false, Some("o teste excedeu 20 s".into())),
    };
    HwStatus {
        encoder: encoder.name.to_owned(),
        working,
        reason,
    }
}

/// Primeira linha de erro significativa do stderr, encurtada.
fn failure_reason(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("falhou sem mensagem");
    let line = line
        .strip_prefix('[')
        .and_then(|l| l.split_once("] "))
        .map_or(line, |(_, r)| r);
    let mut s: String = line.chars().take(120).collect();
    if line.chars().count() > 120 {
        s.push('…');
    }
    s
}

// ---------------------------------------------------------------------------
// Cache

fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("NAGO_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::cache_dir().map(|d| d.join("nago")))
}

pub fn load_cached(dir: &Path, key: &CacheKey) -> Option<Capabilities> {
    let text = std::fs::read_to_string(dir.join("caps.json")).ok()?;
    let caps: Capabilities = serde_json::from_str(&text).ok()?;
    (caps.key == *key).then_some(caps)
}

pub fn save_cache(dir: &Path, caps: &Capabilities) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer_pretty(tmp.as_file(), caps)?;
    tmp.persist(dir.join("caps.json")).map_err(|e| e.error)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Seleção

/// Encoder escolhido e avisos gerados na escolha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub encoder: Encoder,
    pub warnings: Vec<String>,
}

/// Política de seleção: software por padrão; `gpu` → NVENC > QSV > VAAPI com fallback
/// para software; sem nenhum encoder → erro sugerindo codecs alternativos.
pub fn select(caps: &Capabilities, codec: Codec, gpu: bool) -> Result<Selection> {
    let mut warnings = Vec::new();
    let software = Encoder::software(codec);
    let first_hw = || {
        Backend::GPU_PREFERENCE
            .into_iter()
            .map(|b| Encoder::for_backend(codec, b))
            .find(|e| caps.is_usable(e))
    };

    if gpu {
        if let Some(e) = first_hw() {
            return Ok(Selection {
                encoder: e,
                warnings,
            });
        }
        if caps.is_usable(&software) {
            warnings.push(format!(
                "nenhum encoder de GPU funcional para {}; usando o encoder de software {}",
                codec.label(),
                software.name
            ));
            return Ok(Selection {
                encoder: software,
                warnings,
            });
        }
    } else {
        if caps.is_usable(&software) {
            return Ok(Selection {
                encoder: software,
                warnings,
            });
        }
        if let Some(e) = first_hw() {
            warnings.push(format!(
                "o encoder de software {} não está disponível; usando {} ({})",
                software.name,
                e.name,
                e.backend.label()
            ));
            return Ok(Selection {
                encoder: e,
                warnings,
            });
        }
    }

    let alternatives: Vec<&str> = Codec::ALL
        .into_iter()
        .filter(|c| *c != codec)
        .filter(|c| {
            VIDEO_ENCODERS
                .iter()
                .any(|e| e.codec == *c && caps.is_usable(e))
        })
        .map(Codec::cli_name)
        .collect();
    let hint = match alternatives.as_slice() {
        [] => "nenhum outro codec tem encoder disponível; instale um ffmpeg com libsvtav1, libx265 ou libx264".to_owned(),
        alts => format!(
            "tente {}",
            alts.iter().map(|a| format!("--codec {a}")).collect::<Vec<_>>().join(" ou ")
        ),
    };
    Err(Error::Unsupported(format!(
        "não há encoder {} disponível neste ffmpeg; {hint}",
        codec.label()
    )))
}

/// Encoders de áudio/legenda que o planner pode precisar; erro claro se faltar.
pub fn require_encoder(caps: &Capabilities, name: &str, why: &str) -> Result<()> {
    if caps.is_compiled(name) {
        Ok(())
    } else {
        Err(Error::Unsupported(format!(
            "o encoder `{name}` é necessário para {why}, mas não está disponível neste ffmpeg"
        )))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Capacidades simuladas: `compiled` e `working` listam nomes de encoder.
    pub fn fake_caps(compiled: &[&str], working_hw: &[&str]) -> Capabilities {
        let compiled: BTreeSet<String> = compiled.iter().map(|s| s.to_string()).collect();
        let hardware = VIDEO_ENCODERS
            .iter()
            .filter(|e| e.backend.is_hardware() && compiled.contains(e.name))
            .map(|e| {
                let ok = working_hw.contains(&e.name);
                HwStatus {
                    encoder: e.name.into(),
                    working: ok,
                    reason: (!ok).then(|| "dispositivo indisponível".into()),
                }
            })
            .collect();
        Capabilities {
            key: CacheKey {
                ffmpeg: "/usr/bin/ffmpeg".into(),
                mtime_secs: 1,
                version: "9.0.2".into(),
            },
            compiled,
            vmaf: true,
            hardware,
        }
    }

    pub fn full_caps() -> Capabilities {
        fake_caps(
            &[
                "libsvtav1",
                "libx265",
                "libx264",
                "av1_nvenc",
                "hevc_nvenc",
                "h264_nvenc",
                "hevc_vaapi",
                "aac",
                "libopus",
                "libmp3lame",
                "flac",
                "mov_text",
                "webvtt",
                "srt",
                "subrip",
                "alac",
            ],
            &["av1_nvenc", "hevc_nvenc", "h264_nvenc"],
        )
    }

    #[test]
    fn parse_da_lista_de_encoders_do_ffmpeg9() {
        let text = include_str!("../tests/data/encoders_ffmpeg9.txt");
        let set = parse_encoders(text);
        for name in [
            "libsvtav1",
            "libx265",
            "libx264",
            "av1_nvenc",
            "aac",
            "libopus",
            "mov_text",
        ] {
            assert!(set.contains(name), "{name}");
        }
        // linhas da legenda do cabeçalho não viram encoders
        assert!(!set.contains("="));
        assert!(!set.contains("Video"));
    }

    #[test]
    fn filtro_libvmaf() {
        let text = " .. libvmaf           VV->V      Calculate the VMAF\n .. vmafmotion  V->V  x";
        assert!(has_filter(text, "libvmaf"));
        assert!(!has_filter(" .. vmafmotion  V->V  x", "libvmaf"));
    }

    #[test]
    fn motivo_de_falha_resumido() {
        let err = "[hevc_vaapi @ 0x55] No usable encoding entrypoint found for profile VAProfileHEVCMain (17).\nError while opening encoder";
        assert_eq!(
            failure_reason(err),
            "No usable encoding entrypoint found for profile VAProfileHEVCMain (17)."
        );
    }

    #[test]
    fn padrao_e_software() {
        let s = select(&full_caps(), Codec::Av1, false).unwrap();
        assert_eq!(s.encoder.name, "libsvtav1");
        assert!(s.warnings.is_empty());
    }

    #[test]
    fn gpu_prefere_nvenc() {
        let s = select(&full_caps(), Codec::Av1, true).unwrap();
        assert_eq!(s.encoder.name, "av1_nvenc");
    }

    #[test]
    fn gpu_ordem_qsv_antes_de_vaapi() {
        let caps = fake_caps(
            &["libx265", "hevc_qsv", "hevc_vaapi"],
            &["hevc_qsv", "hevc_vaapi"],
        );
        assert_eq!(
            select(&caps, Codec::Hevc, true).unwrap().encoder.name,
            "hevc_qsv"
        );
        let caps = fake_caps(&["libx265", "hevc_qsv", "hevc_vaapi"], &["hevc_vaapi"]);
        assert_eq!(
            select(&caps, Codec::Hevc, true).unwrap().encoder.name,
            "hevc_vaapi"
        );
    }

    #[test]
    fn gpu_indisponivel_cai_para_software_com_aviso() {
        let caps = fake_caps(&["libsvtav1", "av1_vaapi"], &[]);
        let s = select(&caps, Codec::Av1, true).unwrap();
        assert_eq!(s.encoder.name, "libsvtav1");
        assert_eq!(s.warnings.len(), 1);
        assert!(s.warnings[0].contains("software"));
    }

    #[test]
    fn sem_software_usa_hardware_com_aviso() {
        let caps = fake_caps(&["av1_nvenc"], &["av1_nvenc"]);
        let s = select(&caps, Codec::Av1, false).unwrap();
        assert_eq!(s.encoder.name, "av1_nvenc");
        assert_eq!(s.warnings.len(), 1);
    }

    #[test]
    fn sem_encoder_falha_sugerindo_alternativas() {
        let caps = fake_caps(&["libx265", "libx264"], &[]);
        let err = select(&caps, Codec::Av1, false).unwrap_err();
        assert_eq!(err.exit_code(), 1);
        let msg = err.to_string();
        assert!(msg.contains("AV1"), "{msg}");
        assert!(msg.contains("--codec hevc"), "{msg}");
    }

    #[test]
    fn status_para_doctor() {
        let caps = full_caps();
        assert_eq!(
            caps.status(&Encoder::by_name("av1_nvenc").unwrap()),
            EncoderStatus::Available
        );
        assert!(matches!(
            caps.status(&Encoder::by_name("hevc_vaapi").unwrap()),
            EncoderStatus::Unavailable(r) if r.contains("indisponível")
        ));
        assert_eq!(
            caps.status(&Encoder::by_name("av1_qsv").unwrap()),
            EncoderStatus::NotCompiled
        );
    }

    #[test]
    fn cache_invalida_quando_mtime_muda() {
        let dir = tempfile::tempdir().unwrap();
        let caps = full_caps();
        save_cache(dir.path(), &caps).unwrap();
        assert_eq!(load_cached(dir.path(), &caps.key), Some(caps.clone()));
        let mut other = caps.key.clone();
        other.mtime_secs += 1;
        assert_eq!(load_cached(dir.path(), &other), None);
        let mut other = caps.key.clone();
        other.version = "9.0.3".into();
        assert_eq!(load_cached(dir.path(), &other), None);
    }
}
