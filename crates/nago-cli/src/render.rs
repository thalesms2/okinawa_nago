//! Formatação de saída para humanos.

use std::fmt::Write as _;

use humansize::{DECIMAL, format_size};
use nago_core::probe::{MediaInfo, StreamInfo, StreamKind};
use nago_core::timefmt;

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

pub fn media_info(info: &MediaInfo) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}", info.path.display());
    let _ = writeln!(
        out,
        "  container  {} ({})",
        info.container, info.container_long
    );
    let mut line = format!(
        "  duração    {}   tamanho {}",
        timefmt::format(info.duration_secs),
        size(info.size_bytes)
    );
    if let Some(br) = info.bit_rate {
        let _ = write!(line, "   bitrate {}", bitrate(br));
    }
    let _ = writeln!(out, "{line}");
    let _ = writeln!(out);
    for s in &info.streams {
        let _ = writeln!(out, "  {}", stream_line(s));
    }
    out
}

fn kind_label(kind: StreamKind) -> &'static str {
    match kind {
        StreamKind::Video => "vídeo",
        StreamKind::Audio => "áudio",
        StreamKind::Subtitle => "legenda",
        StreamKind::Data => "dados",
        StreamKind::Attachment => "anexo",
    }
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
            parts.push(format!(
                "[{}]",
                serde_json::to_value(hdr).unwrap().as_str().unwrap()
            ));
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

pub fn duration(secs: f64) -> String {
    let s = secs.round() as u64;
    match s {
        0..=59 => format!("{:.1}s", secs),
        60..=3599 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m {:02}s", s / 3600, (s / 60) % 60, s % 60),
    }
}

pub fn report(r: &nago_core::report::Report) -> String {
    use nago_core::report::StreamReport;

    let mut out = String::new();
    let _ = writeln!(out, "✓ {}", r.output.display());
    let _ = writeln!(
        out,
        "  {} → {} ({:+.0}%) em {}",
        size(r.input_size_bytes),
        size(r.output_size_bytes),
        r.size_change_pct,
        duration(r.elapsed_secs)
    );
    if let (Some(req), Some(actual), Some(dur)) =
        (r.requested_start_secs, r.actual_start_secs, r.duration_secs)
    {
        let mut line = format!(
            "  trecho: {} → {} ({})",
            timefmt::format(actual),
            timefmt::format(actual + dur),
            duration(dur)
        );
        if (req - actual).abs() > 0.001 {
            let _ = write!(line, ", pedido a partir de {}", timefmt::format(req));
        }
        let _ = writeln!(out, "{line}");
    }
    if let Some(enc) = &r.encoder {
        let quality = r
            .quality
            .map(|q| format!(" · perfil {}", q.name()))
            .unwrap_or_default();
        let _ = writeln!(out, "  encoder: {enc}{quality}");
    }
    let describe = |s: &StreamReport| -> String {
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
    };
    let streams: Vec<String> = r.streams.iter().map(describe).collect();
    let _ = writeln!(out, "  streams: {}", streams.join(" · "));
    if let (Some(mean), Some(p5)) = (r.vmaf_mean, r.vmaf_p5) {
        let target = r
            .vmaf_target
            .map(|t| {
                let mark = if mean >= t { "✓" } else { "✗" };
                format!(" (meta {t:.0} {mark})")
            })
            .unwrap_or_default();
        let _ = writeln!(out, "  VMAF: média {mean:.2} · p5 {p5:.2}{target}");
    }
    out
}
