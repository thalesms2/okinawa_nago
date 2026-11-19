//! Perfis de qualidade e parâmetros por encoder (design D6).
//!
//! Esta é a única tabela a ajustar na calibração: valores de CRF/CQ por perfil e encoder.

use serde::{Deserialize, Serialize};

use crate::encoders::{Backend, Codec, Encoder, vaapi_device};
use crate::probe::VideoInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Quality {
    VisuallyLossless,
    High,
    Balanced,
    Small,
}

impl Quality {
    pub const ALL: [Quality; 4] = [
        Quality::VisuallyLossless,
        Quality::High,
        Quality::Balanced,
        Quality::Small,
    ];

    /// Meta de VMAF médio documentada para o perfil.
    pub fn vmaf_target(self) -> f64 {
        match self {
            Quality::VisuallyLossless => 97.0,
            Quality::High => 95.0,
            Quality::Balanced => 93.0,
            Quality::Small => 90.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Quality::VisuallyLossless => "visually-lossless",
            Quality::High => "high",
            Quality::Balanced => "balanced",
            Quality::Small => "small",
        }
    }

    fn idx(self) -> usize {
        self as usize
    }
}

/// CRF/CQ por perfil (`[visually-lossless, high, balanced, small]`).
/// Valores calibrados: ver `docs/calibration.md`.
fn quality_value(encoder: &Encoder, q: Quality) -> u32 {
    let table: [u32; 4] = match (encoder.codec, encoder.backend) {
        (Codec::Av1, Backend::Software) => [22, 28, 32, 36],
        (Codec::Hevc, Backend::Software) => [18, 22, 25, 28],
        (Codec::H264, Backend::Software) => [17, 20, 23, 26],
        // AV1 no hardware usa a escala 0–63; HEVC/H.264, 0–51.
        (Codec::Av1, _) => [26, 32, 37, 42],
        (Codec::Hevc, _) => [22, 27, 31, 35],
        (Codec::H264, _) => [20, 24, 28, 32],
    };
    table[q.idx()]
}

pub const SVT_AV1_PRESET: &str = "6";
pub const X26X_PRESET: &str = "slow";

/// Parâmetros de recodificação de um stream de vídeo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VideoEncode {
    pub encoder: String,
    /// Opções por stream (`-chave:N valor`), na ordem.
    pub options: Vec<(String, String)>,
    /// Filtros de vídeo do stream, na ordem.
    pub filters: Vec<String>,
    /// Opções antes do `-i` (ex.: dispositivo VAAPI).
    pub input_options: Vec<String>,
}

/// Monta os parâmetros do encoder para o perfil, preservando cor/HDR de `src`.
pub fn video_encode(encoder: &Encoder, quality: Quality, src: &VideoInfo) -> VideoEncode {
    let q = quality_value(encoder, quality).to_string();
    let ten_bit = encoder.codec != Codec::H264;
    let mut options: Vec<(String, String)> = Vec::new();
    let mut opt = |k: &str, v: &str| options.push((k.to_owned(), v.to_owned()));
    let mut filters = color_filters(src);
    let mut input_options = Vec::new();

    match encoder.backend {
        Backend::Software => {
            opt("pix_fmt", if ten_bit { "yuv420p10le" } else { "yuv420p" });
            match encoder.codec {
                Codec::Av1 => {
                    opt("crf", &q);
                    opt("preset", SVT_AV1_PRESET);
                    opt("g", &gop(src).to_string());
                    let mut params = vec!["tune=0".to_owned()];
                    if src.hdr.is_some() {
                        if let Some(md) = &src.mastering_display {
                            params.push(format!("mastering-display={}", svt_mastering(md)));
                        }
                        if let Some(cl) = &src.content_light {
                            params.push(format!("content-light={},{}", cl.max_cll, cl.max_fall));
                        }
                    }
                    opt("svtav1-params", &params.join(":"));
                }
                Codec::Hevc => {
                    opt("crf", &q);
                    opt("preset", X26X_PRESET);
                    let mut params = vec!["log-level=error".to_owned()];
                    if src.hdr.is_some() {
                        params.push("repeat-headers=1".into());
                        if let Some(md) = &src.mastering_display {
                            params.push("hdr10=1".into());
                            params.push("hdr10-opt=1".into());
                            params.push(format!("master-display={}", x265_mastering(md)));
                        }
                        if let Some(cl) = &src.content_light {
                            params.push(format!("max-cll={},{}", cl.max_cll, cl.max_fall));
                        }
                    }
                    opt("x265-params", &params.join(":"));
                }
                Codec::H264 => {
                    opt("crf", &q);
                    opt("preset", X26X_PRESET);
                }
            }
        }
        Backend::Nvenc => {
            opt("pix_fmt", if ten_bit { "p010le" } else { "yuv420p" });
            opt("preset", "p6");
            opt("tune", "hq");
            opt("rc", "vbr");
            opt("cq", &q);
            opt("b", "0");
            opt("multipass", "qres");
            opt("spatial-aq", "1");
            opt("temporal-aq", "1");
            opt("rc-lookahead", "32");
            opt("g", &gop(src).to_string());
        }
        Backend::Qsv => {
            opt("pix_fmt", if ten_bit { "p010le" } else { "nv12" });
            opt("preset", "slower");
            opt("global_quality", &q);
            opt("g", &gop(src).to_string());
        }
        Backend::Vaapi => {
            input_options.extend(["-vaapi_device".to_owned(), vaapi_device()]);
            filters.push(format!("format={}", if ten_bit { "p010" } else { "nv12" }));
            filters.push("hwupload".into());
            opt("rc_mode", "CQP");
            opt("qp", &q);
            opt("g", &gop(src).to_string());
        }
    }

    VideoEncode {
        encoder: encoder.name.to_owned(),
        options,
        filters,
        input_options,
    }
}

/// GOP de ~10 s (facilita seek sem custar muito em compressão).
fn gop(src: &VideoInfo) -> u32 {
    let fps = src.fps.unwrap_or(24.0);
    ((fps * 10.0).round() as u32).clamp(1, 600)
}

/// `setparams` com as propriedades de cor conhecidas da fonte (design D8).
fn color_filters(src: &VideoInfo) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(p) = &src.color_primaries {
        parts.push(format!("color_primaries={p}"));
    }
    if let Some(t) = &src.color_transfer {
        parts.push(format!("color_trc={t}"));
    }
    if let Some(s) = &src.color_space {
        parts.push(format!("colorspace={s}"));
    }
    if let Some(r) = &src.color_range {
        parts.push(format!("range={r}"));
    }
    if parts.is_empty() {
        Vec::new()
    } else {
        vec![format!("setparams={}", parts.join(":"))]
    }
}

fn svt_mastering(md: &crate::probe::MasteringDisplay) -> String {
    format!(
        "G({:.4},{:.4})B({:.4},{:.4})R({:.4},{:.4})WP({:.4},{:.4})L({:.4},{:.4})",
        md.green.0,
        md.green.1,
        md.blue.0,
        md.blue.1,
        md.red.0,
        md.red.1,
        md.white_point.0,
        md.white_point.1,
        md.max_luminance,
        md.min_luminance
    )
}

fn x265_mastering(md: &crate::probe::MasteringDisplay) -> String {
    let c = |v: f64| (v * 50000.0).round() as u64;
    let l = |v: f64| (v * 10000.0).round() as u64;
    format!(
        "G({},{})B({},{})R({},{})WP({},{})L({},{})",
        c(md.green.0),
        c(md.green.1),
        c(md.blue.0),
        c(md.blue.1),
        c(md.red.0),
        c(md.red.1),
        c(md.white_point.0),
        c(md.white_point.1),
        l(md.max_luminance),
        l(md.min_luminance)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoders::VIDEO_ENCODERS;
    use crate::probe::{ContentLight, HdrKind, MasteringDisplay};

    pub fn sdr_1080p() -> VideoInfo {
        VideoInfo {
            width: 1920,
            height: 1080,
            fps: Some(30.0),
            pix_fmt: Some("yuv420p".into()),
            bit_depth: 8,
            color_primaries: Some("bt709".into()),
            color_transfer: Some("bt709".into()),
            color_space: Some("bt709".into()),
            color_range: Some("tv".into()),
            hdr: None,
            dolby_vision: false,
            rotation: 0,
            mastering_display: None,
            content_light: None,
        }
    }

    pub fn hdr10() -> VideoInfo {
        VideoInfo {
            pix_fmt: Some("yuv420p10le".into()),
            bit_depth: 10,
            color_primaries: Some("bt2020".into()),
            color_transfer: Some("smpte2084".into()),
            color_space: Some("bt2020nc".into()),
            hdr: Some(HdrKind::Hdr10),
            mastering_display: Some(MasteringDisplay {
                red: (0.68, 0.32),
                green: (0.265, 0.69),
                blue: (0.15, 0.06),
                white_point: (0.3127, 0.329),
                min_luminance: 0.005,
                max_luminance: 1000.0,
            }),
            content_light: Some(ContentLight {
                max_cll: 1000,
                max_fall: 400,
            }),
            ..sdr_1080p()
        }
    }

    fn as_args(v: &VideoEncode) -> Vec<String> {
        let mut out = v.input_options.clone();
        out.push(format!("-c:v {}", v.encoder));
        out.extend(v.options.iter().map(|(k, val)| format!("-{k} {val}")));
        if !v.filters.is_empty() {
            out.push(format!("-vf {}", v.filters.join(",")));
        }
        out
    }

    #[test]
    fn snapshot_de_todos_os_perfis_e_encoders() {
        let src = sdr_1080p();
        let mut all = Vec::new();
        for enc in VIDEO_ENCODERS {
            for q in Quality::ALL {
                all.push(format!(
                    "{} / {}: {}",
                    enc.name,
                    q.name(),
                    as_args(&video_encode(&enc, q, &src)).join(" ")
                ));
            }
        }
        insta::assert_snapshot!("perfis_sdr", all.join("\n"));
    }

    #[test]
    fn snapshot_hdr10_svt_e_x265() {
        let src = hdr10();
        let lines: Vec<String> = ["libsvtav1", "libx265", "av1_nvenc"]
            .iter()
            .map(|n| {
                let e = Encoder::by_name(n).unwrap();
                format!(
                    "{n}: {}",
                    as_args(&video_encode(&e, Quality::High, &src)).join(" ")
                )
            })
            .collect();
        insta::assert_snapshot!("perfis_hdr10", lines.join("\n"));
    }

    #[test]
    fn mastering_display_nos_formatos_dos_encoders() {
        let md = hdr10().mastering_display.unwrap();
        assert_eq!(
            x265_mastering(&md),
            "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,50)"
        );
        assert_eq!(
            svt_mastering(&md),
            "G(0.2650,0.6900)B(0.1500,0.0600)R(0.6800,0.3200)WP(0.3127,0.3290)L(1000.0000,0.0050)"
        );
    }

    #[test]
    fn metas_de_vmaf_documentadas() {
        assert_eq!(Quality::VisuallyLossless.vmaf_target(), 97.0);
        assert_eq!(Quality::High.vmaf_target(), 95.0);
        assert_eq!(Quality::Balanced.vmaf_target(), 93.0);
        assert_eq!(Quality::Small.vmaf_target(), 90.0);
    }
}
