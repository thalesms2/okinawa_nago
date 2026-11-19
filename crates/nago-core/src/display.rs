//! Formatação para humanos, compartilhada pela CLI e pela TUI.
//!
//! As funções devolvem [`Line`]s com segmentos marcados por [`Tone`], sem códigos
//! ANSI: a CLI junta o texto e a TUI aplica estilos por tom.

use std::fmt::Write as _;

use humansize::{DECIMAL, format_size};

use crate::probe::{MediaInfo, StreamInfo, StreamKind};
use crate::report::{Report, StreamReport};
use crate::timefmt;

/// Papel visual de um trecho de texto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Normal,
    /// Rótulo de um campo (`duração`, `encoder:`).
    Label,
    /// Caminho de arquivo.
    Path,
    /// Resultado bom (meta atingida, redução de tamanho).
    Good,
    /// Resultado ruim (meta não atingida, arquivo maior).
    Bad,
    /// Informação secundária.
    Muted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seg {
    pub text: String,
    pub tone: Tone,
}

/// Uma linha de texto formada por segmentos.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Line(pub Vec<Seg>);

impl Line {
    pub fn new() -> Line {
        Line(Vec::new())
    }

    pub fn push(mut self, tone: Tone, text: impl Into<String>) -> Line {
        self.0.push(Seg {
            text: text.into(),
            tone,
        });
        self
    }

    pub fn normal(self, text: impl Into<String>) -> Line {
        self.push(Tone::Normal, text)
    }

    pub fn label(self, text: impl Into<String>) -> Line {
        self.push(Tone::Label, text)
    }

    /// O texto da linha, sem marcações.
    pub fn text(&self) -> String {
        self.0.iter().map(|s| s.text.as_str()).collect()
    }
}

/// Junta as linhas em texto, cada uma terminada por `\n`.
pub fn to_text(lines: &[Line]) -> String {
    let mut out = String::new();
    for l in lines {
        out.push_str(&l.text());
        out.push('\n');
    }
    out
}

pub fn size(bytes: u64) -> String {
    format_size(bytes, DECIMAL)
}

pub fn bitrate(bps: u64) -> String {
    if bps >= 1_000_000 {
        format!("{:.1} Mb/s", bps as f64 / 1e6)
    } else {
        format!("{} kb/s", bps / 1000)
    }
}

/// Duração de execução legível (`4.2s`, `1m 05s`, `1h 02m 03s`).
pub fn duration(secs: f64) -> String {
    let s = secs.round() as u64;
    match s {
        0..=59 => format!("{:.1}s", secs),
        60..=3599 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m {:02}s", s / 3600, (s / 60) % 60, s % 60),
    }
}

pub fn kind_label(kind: StreamKind) -> &'static str {
    match kind {
        StreamKind::Video => "vídeo",
        StreamKind::Audio => "áudio",
        StreamKind::Subtitle => "legenda",
        StreamKind::Data => "dados",
        StreamKind::Attachment => "anexo",
    }
}

/// Metadados no formato do `nago info`.
pub fn media_info(info: &MediaInfo) -> Vec<Line> {
    let mut lines = vec![
        Line::new().push(Tone::Path, info.path.display().to_string()),
        Line::new()
            .label("  container  ")
            .normal(format!("{} ({})", info.container, info.container_long)),
    ];
    let mut l = Line::new()
        .label("  duração    ")
        .normal(timefmt::format(info.duration_secs))
        .label("   tamanho ")
        .normal(size(info.size_bytes));
    if let Some(br) = info.bit_rate {
        l = l.label("   bitrate ").normal(bitrate(br));
    }
    lines.push(l);
    lines.push(Line::new());
    for s in &info.streams {
        lines.push(Line::new().normal(format!("  {}", stream_line(s))));
    }
    lines
}

pub fn stream_line(s: &StreamInfo) -> String {
    let mut parts: Vec<String> = Vec::new();
    let codec = match &s.profile {
        Some(p) => format!("{} ({p})", s.codec),
        None => s.codec.clone(),
    };
    parts.push(codec);
    if let Some(v) = &s.video {
        parts.push(format!("{}x{}", v.width, v.height));
        if let Some(fps) = v.fps {
            let fps = if (fps - fps.round()).abs() < 0.01 {
                format!("{fps:.0}")
            } else {
                format!("{fps:.3}")
            };
            parts.push(format!("{fps} fps"));
        }
        parts.push(format!("{}-bit", v.bit_depth));
        if let Some(p) = &v.color_primaries {
            parts.push(p.clone());
        }
        if v.rotation != 0 {
            parts.push(format!("rotação {}°", v.rotation));
        }
        if let Some(hdr) = v.hdr {
            let name = serde_json::to_value(hdr)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            parts.push(format!("[{name}]"));
        }
        if v.dolby_vision {
            parts.push("[Dolby Vision]".into());
        }
    }
    if let Some(a) = &s.audio {
        parts.push(match a.channel_layout.as_deref() {
            Some(l) => format!("{} canais ({l})", a.channels),
            None => format!("{} canais", a.channels),
        });
        if let Some(sr) = a.sample_rate {
            parts.push(format!("{} kHz", sr as f64 / 1000.0));
        }
    }
    if let Some(br) = s.bit_rate {
        parts.push(bitrate(br));
    }
    if let Some(l) = &s.language {
        parts.push(l.clone());
    }
    if let Some(t) = &s.title {
        parts.push(format!("\"{t}\""));
    }
    if s.attached_pic {
        parts.push("[capa]".into());
    }
    if s.default {
        parts.push("[padrão]".into());
    }
    if s.forced {
        parts.push("[forçada]".into());
    }
    format!(
        "#{:<2} {:<8} {}",
        s.index,
        kind_label(s.kind),
        parts.join("  ")
    )
}

/// Ação de um stream no relatório (`#1 áudio aac copiado`).
pub fn describe_stream(s: &StreamReport) -> String {
    let kind = kind_label(s.kind);
    match s.action {
        "copy" => format!("#{} {kind} {} copiado", s.index, s.codec_in),
        "transcode" => format!(
            "#{} {kind} {} → {}",
            s.index,
            s.codec_in,
            s.codec_out.as_deref().unwrap_or("?")
        ),
        _ => format!("#{} {kind} {} descartado", s.index, s.codec_in),
    }
}

/// Relatório final de um job.
pub fn report(r: &Report) -> Vec<Line> {
    let mut lines = vec![
        Line::new()
            .push(Tone::Good, "✓ ")
            .push(Tone::Path, r.output.display().to_string()),
    ];
    let change_tone = if r.size_change_pct < 0.0 {
        Tone::Good
    } else {
        Tone::Bad
    };
    lines.push(
        Line::new()
            .normal(format!(
                "  {} → {} ",
                size(r.input_size_bytes),
                size(r.output_size_bytes)
            ))
            .push(change_tone, format!("({:+.0}%)", r.size_change_pct))
            .normal(format!(" em {}", duration(r.elapsed_secs))),
    );
    if let (Some(req), Some(actual), Some(dur)) =
        (r.requested_start_secs, r.actual_start_secs, r.duration_secs)
    {
        let mut text = format!(
            "{} → {} ({})",
            timefmt::format(actual),
            timefmt::format(actual + dur),
            duration(dur)
        );
        if (req - actual).abs() > 0.001 {
            let _ = write!(text, ", pedido a partir de {}", timefmt::format(req));
        }
        lines.push(Line::new().label("  trecho: ").normal(text));
    }
    if let Some(enc) = &r.encoder {
        let quality = r
            .quality
            .map(|q| format!(" · perfil {}", q.name()))
            .unwrap_or_default();
        lines.push(
            Line::new()
                .label("  encoder: ")
                .normal(format!("{enc}{quality}")),
        );
    }
    let streams: Vec<String> = r.streams.iter().map(describe_stream).collect();
    lines.push(Line::new().label("  streams: ").normal(streams.join(" · ")));
    if let (Some(mean), Some(p5)) = (r.vmaf_mean, r.vmaf_p5) {
        let mut l = Line::new()
            .label("  VMAF: ")
            .normal(format!("média {mean:.2} · p5 {p5:.2}"));
        if let Some(t) = r.vmaf_target {
            let (mark, tone) = if mean >= t {
                ("✓", Tone::Good)
            } else {
                ("✗", Tone::Bad)
            };
            l = l.push(tone, format!(" (meta {t:.0} {mark})"));
        }
        lines.push(l);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duracoes() {
        assert_eq!(duration(4.24), "4.2s");
        assert_eq!(duration(65.0), "1m 05s");
        assert_eq!(duration(3723.0), "1h 02m 03s");
    }

    #[test]
    fn texto_da_linha_junta_segmentos() {
        let l = Line::new().label("a: ").push(Tone::Good, "b");
        assert_eq!(l.text(), "a: b");
        assert_eq!(to_text(&[l, Line::new()]), "a: b\n\n");
    }
}
