//! Planejamento puro (MediaInfo + opções → Plan) e montagem da linha de comando
//! (Plan → argumentos do ffmpeg). Nada aqui executa processos (design D2).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::caps::{self, Capabilities};
use crate::container::{Compat, Container};
use crate::encoders::{Codec, Encoder};
use crate::probe::{MediaInfo, StreamInfo, StreamKind};
use crate::profiles::{self, Quality};
use crate::timefmt;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Encode {
    pub encoder: String,
    pub options: Vec<(String, String)>,
    pub filters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum Action {
    Copy,
    Transcode(Encode),
    Drop { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StreamPlan {
    pub index: usize,
    pub kind: StreamKind,
    pub codec_in: String,
    pub action: Action,
    /// Por que o stream foi recodificado (quando não foi pedido explicitamente).
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub input: PathBuf,
    /// Opções antes do `-i` (seek, dispositivo de hardware).
    pub input_options: Vec<String>,
    pub streams: Vec<StreamPlan>,
    /// Muxer explícito; `None` deixa o ffmpeg inferir pela extensão.
    pub muxer: Option<&'static str>,
    /// Extensão do arquivo de saída (sem ponto).
    pub ext: String,
    pub output_options: Vec<String>,
    /// Duração de mídia esperada na saída (para o progresso).
    pub expected_duration: f64,
    pub video_encoder: Option<String>,
    pub quality: Option<Quality>,
    pub warnings: Vec<String>,
}

impl Plan {
    pub fn transcodes_anything(&self) -> bool {
        self.streams
            .iter()
            .any(|s| matches!(s.action, Action::Transcode(_)))
    }
}

/// Monta os argumentos do ffmpeg (sem os globais que o runner acrescenta).
pub fn build_args(plan: &Plan, output: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    let mut push = |s: &str| args.push(OsString::from(s));
    for o in &plan.input_options {
        push(o);
    }
    push("-i");
    args.push(plan.input.clone().into_os_string());

    let mut push = |s: String| args.push(OsString::from(s));
    let kept = plan
        .streams
        .iter()
        .filter(|s| !matches!(s.action, Action::Drop { .. }));
    for (out_idx, s) in kept.enumerate() {
        push("-map".into());
        push(format!("0:{}", s.index));
        match &s.action {
            Action::Copy => {
                push(format!("-c:{out_idx}"));
                push("copy".into());
            }
            Action::Transcode(enc) => {
                push(format!("-c:{out_idx}"));
                push(enc.encoder.clone());
                for (k, v) in &enc.options {
                    push(format!("-{k}:{out_idx}"));
                    push(v.clone());
                }
                if !enc.filters.is_empty() {
                    push(format!("-filter:{out_idx}"));
                    push(enc.filters.join(","));
                }
            }
            Action::Drop { .. } => unreachable!(),
        }
    }
    for o in &plan.output_options {
        push(o.clone());
    }
    if let Some(m) = plan.muxer {
        push("-f".into());
        push(m.into());
    }
    args.push(output.as_os_str().to_owned());
    args
}

// ---------------------------------------------------------------------------
// Áudio e legendas

/// Parâmetros para recodificar áudio sem perda perceptível.
pub fn audio_encode(encoder: &str, stream: &StreamInfo, bitrate_kbps: Option<u32>) -> Encode {
    let channels = stream
        .audio
        .as_ref()
        .map(|a| a.channels)
        .unwrap_or(2)
        .max(1);
    let mut options = Vec::new();
    let mut filters = Vec::new();
    match encoder {
        "aac" => {
            let kbps = bitrate_kbps.unwrap_or((128 * channels).min(640));
            options.push(("b".into(), format!("{kbps}k")));
        }
        "libopus" => {
            let kbps = bitrate_kbps.unwrap_or((80 * channels).min(512));
            options.push(("b".into(), format!("{kbps}k")));
            if channels > 2 {
                options.push(("mapping_family".into(), "1".into()));
                filters.push("aformat=channel_layouts=7.1|5.1|stereo".into());
            }
        }
        "libmp3lame" => {
            let kbps = bitrate_kbps.unwrap_or(320);
            options.push(("b".into(), format!("{kbps}k")));
            if channels > 2 {
                options.push(("ac".into(), "2".into()));
            }
        }
        _ => {}
    }
    Encode {
        encoder: encoder.to_owned(),
        options,
        filters,
    }
}

fn simple_encode(encoder: &str) -> Encode {
    Encode {
        encoder: encoder.to_owned(),
        options: Vec::new(),
        filters: Vec::new(),
    }
}

/// Transforma a decisão de compatibilidade em ação para áudio/legenda/capa.
fn compat_action(
    caps: &Capabilities,
    stream: &StreamInfo,
    compat: Compat,
    container: Container,
    bitrate: Option<u32>,
) -> Result<(Action, Option<String>)> {
    Ok(match compat {
        Compat::Copy => (Action::Copy, None),
        Compat::Drop(reason) => (Action::Drop { reason }, None),
        Compat::Transcode(enc) => {
            caps::require_encoder(
                caps,
                enc,
                &format!("gravar {} em {}", stream.codec, container.ext()),
            )?;
            let encode = if stream.kind == StreamKind::Audio {
                audio_encode(enc, stream, bitrate)
            } else {
                simple_encode(enc)
            };
            (
                Action::Transcode(encode),
                Some(format!(
                    "{} não é suportado em {}",
                    stream.codec,
                    container.ext()
                )),
            )
        }
    })
}

fn same_container(info: &MediaInfo, target: Container) -> bool {
    Container::from_input(&info.container, info.extension().as_deref()) == Some(target)
}

/// Decisão padrão para streams que não são o vídeo principal.
fn secondary_stream(
    caps: &Capabilities,
    info: &MediaInfo,
    s: &StreamInfo,
    target: Container,
    compat_audio: bool,
    audio_bitrate: Option<u32>,
    audio_reencode: Option<&'static str>,
) -> Result<(Action, Option<String>)> {
    match s.kind {
        StreamKind::Audio => {
            if let Some(enc) = audio_reencode {
                caps::require_encoder(caps, enc, "recodificar o áudio")?;
                return Ok((
                    Action::Transcode(audio_encode(enc, s, audio_bitrate)),
                    Some("--audio-bitrate".into()),
                ));
            }
            compat_action(
                caps,
                s,
                target.audio(&s.codec, compat_audio),
                target,
                audio_bitrate,
            )
        }
        StreamKind::Subtitle => compat_action(caps, s, target.subtitle(&s.codec), target, None),
        StreamKind::Attachment => Ok((
            match target.attachment() {
                Compat::Copy => Action::Copy,
                Compat::Drop(reason) => Action::Drop { reason },
                Compat::Transcode(_) => unreachable!(),
            },
            None,
        )),
        StreamKind::Video if s.attached_pic => {
            compat_action(caps, s, target.cover(&s.codec), target, None)
        }
        StreamKind::Data => Ok(if same_container(info, target) {
            (Action::Copy, None)
        } else {
            (
                Action::Drop {
                    reason: format!(
                        "stream de dados ({}) não é transferível para {}",
                        s.codec,
                        target.ext()
                    ),
                },
                None,
            )
        }),
        StreamKind::Video => unreachable!("vídeo principal é tratado pelo chamador"),
    }
}

/// Avisos para streams descartados.
fn drop_warnings(streams: &[StreamPlan]) -> Vec<String> {
    streams
        .iter()
        .filter_map(|s| match &s.action {
            Action::Drop { reason } => Some(format!(
                "stream #{} ({}) descartado: {reason}",
                s.index, s.codec_in
            )),
            _ => None,
        })
        .collect()
}

fn mp4_output_options(target: Container, streams: &[StreamPlan]) -> Vec<String> {
    let mut opts = Vec::new();
    if target.is_isobmff() {
        opts.extend(["-movflags".to_owned(), "+faststart".to_owned()]);
        // Tag hvc1 para HEVC em MP4/MOV: necessário para reprodução em dispositivos Apple.
        let kept = streams
            .iter()
            .filter(|s| !matches!(s.action, Action::Drop { .. }));
        for (i, s) in kept.enumerate() {
            let is_hevc = match &s.action {
                Action::Transcode(e) => {
                    Encoder::by_name(&e.encoder).is_some_and(|e| e.codec == Codec::Hevc)
                }
                Action::Copy => s.codec_in == "hevc",
                _ => false,
            };
            if is_hevc && s.kind == StreamKind::Video {
                opts.extend([format!("-tag:{i}"), "hvc1".to_owned()]);
            }
        }
    }
    opts
}

fn video_transcode(encoder: &Encoder, quality: Quality, s: &StreamInfo) -> (Encode, Vec<String>) {
    let v = s.video.as_ref().expect("stream de vídeo tem VideoInfo");
    let ve = profiles::video_encode(encoder, quality, v);
    let mut options = ve.options;
    // Frame rate variável (celular/tela): não duplicar nem descartar frames.
    options.push(("fps_mode".into(), "passthrough".into()));
    (
        Encode {
            encoder: ve.encoder,
            options,
            filters: ve.filters,
        },
        ve.input_options,
    )
}

// ---------------------------------------------------------------------------
// compress

#[derive(Debug, Clone)]
pub struct CompressOptions {
    pub quality: Quality,
    pub codec: Codec,
    pub gpu: bool,
    pub audio_bitrate: Option<u32>,
    /// Container pedido via extensão de `-o`, se houver.
    pub container: Option<Container>,
}

/// Container de saída da compressão: o pedido, o da entrada (se suportar o codec) ou mkv.
pub fn compress_container(info: &MediaInfo, opts: &CompressOptions) -> Result<Container> {
    if let Some(c) = opts.container {
        if c.is_audio_only() || !c.supports_video_codec(opts.codec) {
            return Err(Error::usage(format!(
                "{} não suporta vídeo {}; use .mkv, .mp4 ou outro --codec",
                c.ext(),
                opts.codec.label()
            )));
        }
        return Ok(c);
    }
    Ok(
        Container::from_input(&info.container, info.extension().as_deref())
            .filter(|c| c.supports_video_codec(opts.codec))
            .unwrap_or(Container::Mkv),
    )
}

pub fn plan_compress(
    info: &MediaInfo,
    caps: &Capabilities,
    opts: &CompressOptions,
) -> Result<Plan> {
    let primary = info
        .primary_video()
        .ok_or_else(|| Error::usage("o arquivo não tem stream de vídeo para comprimir"))?;
    let pv = primary.video.as_ref().unwrap();
    if pv.hdr.is_some() && !opts.codec.supports_hdr() {
        return Err(Error::usage(format!(
            "o vídeo é HDR e {} não preservará o HDR (a saída ficaria com cores erradas); use --codec av1 ou --codec hevc",
            opts.codec.label()
        )));
    }
    let target = compress_container(info, opts)?;
    let selection = caps::select(caps, opts.codec, opts.gpu)?;
    let mut warnings = selection.warnings.clone();
    if pv.dolby_vision {
        warnings.push(
            "o vídeo tem Dolby Vision: a camada DV não é preservada na recodificação; a saída será HDR10".into(),
        );
    }
    if opts.codec == Codec::H264 && pv.bit_depth > 8 {
        warnings
            .push("H.264 será gravado em 8-bit; para manter 10-bit use --codec av1 ou hevc".into());
    }

    let audio_reencode = opts.audio_bitrate.map(|_| match target {
        Container::Mov => "aac",
        _ => "libopus",
    });

    let mut input_options = Vec::new();
    let mut streams = Vec::new();
    for s in &info.streams {
        let (action, note) = if s.kind == StreamKind::Video && !s.attached_pic {
            let (enc, input_opts) = video_transcode(&selection.encoder, opts.quality, s);
            if input_options.is_empty() {
                input_options = input_opts;
            }
            (Action::Transcode(enc), None)
        } else {
            secondary_stream(
                caps,
                info,
                s,
                target,
                false,
                opts.audio_bitrate,
                audio_reencode,
            )?
        };
        streams.push(StreamPlan {
            index: s.index,
            kind: s.kind,
            codec_in: s.codec.clone(),
            action,
            note,
        });
    }
    warnings.extend(drop_warnings(&streams));
    let output_options = mp4_output_options(target, &streams);
    Ok(Plan {
        input: info.path.clone(),
        input_options,
        streams,
        muxer: Some(target.muxer()),
        ext: target.ext().to_owned(),
        output_options,
        expected_duration: info.duration_secs,
        video_encoder: Some(selection.encoder.name.to_owned()),
        quality: Some(opts.quality),
        warnings,
    })
}

// ---------------------------------------------------------------------------
// convert

#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub target: Container,
    /// Faixa de áudio (1 = primeira) para formatos somente de áudio.
    pub audio_track: Option<usize>,
    pub compat: bool,
}

pub fn plan_convert(info: &MediaInfo, caps: &Capabilities, opts: &ConvertOptions) -> Result<Plan> {
    let target = opts.target;
    let mut warnings = Vec::new();
    let mut streams = Vec::new();
    let mut video_encoder = None;
    let mut quality = None;
    let mut input_options = Vec::new();

    if target.is_audio_only() {
        let audios: Vec<&StreamInfo> = info.audio_streams().collect();
        if audios.is_empty() {
            return Err(Error::usage(
                "o arquivo não tem faixa de áudio para extrair",
            ));
        }
        let n = opts.audio_track.unwrap_or(1);
        let chosen = *audios.get(n.wrapping_sub(1)).ok_or_else(|| {
            Error::usage(format!(
                "--audio-track {n} inválido: o arquivo tem {} faixa(s) de áudio",
                audios.len()
            ))
        })?;
        for s in &info.streams {
            if s.index != chosen.index {
                continue;
            }
            let (action, note) =
                compat_action(caps, s, target.audio(&s.codec, opts.compat), target, None)?;
            streams.push(StreamPlan {
                index: s.index,
                kind: s.kind,
                codec_in: s.codec.clone(),
                action,
                note,
            });
        }
    } else {
        if opts.audio_track.is_some() {
            warnings
                .push("--audio-track só se aplica a formatos somente de áudio; ignorado".into());
        }
        for s in &info.streams {
            let (action, note) = if s.kind == StreamKind::Video && !s.attached_pic {
                match target.video(&s.codec) {
                    Compat::Copy => (Action::Copy, None),
                    _ => {
                        let sel = caps::select(caps, Codec::Av1, false)?;
                        warnings.extend(sel.warnings);
                        let (enc, input_opts) = video_transcode(&sel.encoder, Quality::High, s);
                        if input_options.is_empty() {
                            input_options = input_opts;
                        }
                        video_encoder = Some(sel.encoder.name.to_owned());
                        quality = Some(Quality::High);
                        (
                            Action::Transcode(enc),
                            Some(format!(
                                "vídeo {} não é suportado em {}",
                                s.codec,
                                target.ext()
                            )),
                        )
                    }
                }
            } else {
                secondary_stream(caps, info, s, target, opts.compat, None, None)?
            };
            streams.push(StreamPlan {
                index: s.index,
                kind: s.kind,
                codec_in: s.codec.clone(),
                action,
                note,
            });
        }
    }
    warnings.extend(drop_warnings(&streams));
    let output_options = mp4_output_options(target, &streams);
    Ok(Plan {
        input: info.path.clone(),
        input_options,
        streams,
        muxer: Some(target.muxer()),
        ext: target.ext().to_owned(),
        output_options,
        expected_duration: info.duration_secs,
        video_encoder,
        quality,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// cut

/// Intervalo validado (segundos relativos ao início do arquivo).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutRange {
    pub start: f64,
    pub end: f64,
}

/// Valida `--from/--to/--duration` contra a duração do arquivo.
/// Devolve o intervalo e um aviso se o fim passou da duração.
pub fn resolve_cut_range(
    duration: f64,
    from: Option<f64>,
    to: Option<f64>,
    length: Option<f64>,
) -> Result<(CutRange, Option<String>)> {
    if to.is_some() && length.is_some() {
        return Err(Error::usage("use --to ou --duration, não os dois"));
    }
    let start = from.unwrap_or(0.0);
    if start >= duration {
        return Err(Error::usage(format!(
            "o início {} está além da duração do arquivo ({})",
            timefmt::format(start),
            timefmt::format(duration)
        )));
    }
    let mut end = match (to, length) {
        (Some(t), _) => t,
        (_, Some(l)) => {
            if l <= 0.0 {
                return Err(Error::usage("--duration deve ser maior que zero"));
            }
            start + l
        }
        _ => duration,
    };
    if end <= start {
        return Err(Error::usage(format!(
            "o início ({}) deve ser anterior ao fim ({})",
            timefmt::format(start),
            timefmt::format(end)
        )));
    }
    let mut warning = None;
    if end > duration + 0.001 {
        warning = Some(format!(
            "o fim pedido ({}) passa da duração do arquivo; o corte vai até o fim ({})",
            timefmt::format(end),
            timefmt::format(duration)
        ));
        end = duration;
    }
    Ok((CutRange { start, end }, warning))
}

#[derive(Debug, Clone)]
pub struct CutOptions {
    pub range: CutRange,
    pub exact: bool,
    /// Keyframe anterior ou igual ao início, calculado pelo chamador. No modo rápido é
    /// o início real; no exato, só o ponto de seek de entrada.
    pub keyframe_start: Option<f64>,
    /// Container de saída pedido via `-o` (diferente do da entrada).
    pub container: Option<Container>,
}

/// Folga para que o seek caia exatamente no keyframe calculado.
const SEEK_EPSILON: f64 = 0.0005;

pub fn plan_cut(info: &MediaInfo, caps: &Capabilities, opts: &CutOptions) -> Result<Plan> {
    let input_container = Container::from_input(&info.container, info.extension().as_deref());
    let target = opts.container.or(input_container);
    let ext = target
        .map(|c| c.ext().to_owned())
        .or_else(|| info.extension())
        .unwrap_or_else(|| "mkv".into());
    let mut warnings = Vec::new();
    let mut streams = Vec::new();
    let mut input_options = Vec::new();
    let mut video_encoder = None;
    let mut quality = None;

    // O seek de entrada sempre cai no keyframe anterior (rápido e sem pré-roll de
    // pacotes copiados). No modo exato, um seek de saída descarta com precisão o que
    // vem antes de `start` — tanto frames decodificados quanto pacotes de áudio
    // copiados, mantendo tudo sincronizado.
    let keyframe = opts
        .keyframe_start
        .unwrap_or(opts.range.start)
        .min(opts.range.start);
    let input_seek = if keyframe > 0.0 {
        keyframe + SEEK_EPSILON
    } else {
        0.0
    };
    let (output_seek, duration) = if opts.exact {
        (
            (opts.range.start - input_seek).max(0.0),
            opts.range.end - opts.range.start,
        )
    } else {
        (0.0, opts.range.end - keyframe)
    };

    for s in &info.streams {
        let is_main_video = s.kind == StreamKind::Video && !s.attached_pic;
        let (action, note) = if is_main_video && opts.exact {
            // Mesmo codec da entrada quando houver encoder; senão AV1.
            let codec = Codec::from_ffprobe(&s.codec).unwrap_or(Codec::Av1);
            let sel = caps::select(caps, codec, false)
                .or_else(|_| caps::select(caps, Codec::Av1, false))?;
            let (enc, input_opts) = video_transcode(&sel.encoder, Quality::VisuallyLossless, s);
            input_options.extend(input_opts);
            video_encoder = Some(sel.encoder.name.to_owned());
            quality = Some(Quality::VisuallyLossless);
            (Action::Transcode(enc), Some("--exact".into()))
        } else if let (Some(t), true) = (target, target != input_container) {
            // -o com outro container: aplica a matriz de compatibilidade.
            if is_main_video {
                match t.video(&s.codec) {
                    Compat::Copy => (Action::Copy, None),
                    _ => {
                        return Err(Error::usage(format!(
                            "vídeo {} não cabe em {} sem recodificar; use --exact, outro -o ou `nago convert` depois",
                            s.codec,
                            t.ext()
                        )));
                    }
                }
            } else {
                secondary_stream(caps, info, s, t, false, None, None)?
            }
        } else {
            (Action::Copy, None)
        };
        streams.push(StreamPlan {
            index: s.index,
            kind: s.kind,
            codec_in: s.codec.clone(),
            action,
            note,
        });
    }
    warnings.extend(drop_warnings(&streams));

    if input_seek > 0.0 {
        input_options.extend(["-ss".to_owned(), timefmt::ffmpeg_arg(input_seek)]);
    }
    let mut output_options = Vec::new();
    if output_seek > 0.0 {
        output_options.extend(["-ss".to_owned(), timefmt::ffmpeg_arg(output_seek)]);
    }
    output_options.extend([
        "-t".to_owned(),
        timefmt::ffmpeg_arg(duration),
        "-avoid_negative_ts".to_owned(),
        "make_zero".to_owned(),
    ]);
    if let Some(t) = target {
        output_options.extend(mp4_output_options(t, &streams));
    }
    Ok(Plan {
        input: info.path.clone(),
        input_options,
        streams,
        muxer: target.map(Container::muxer),
        ext,
        output_options,
        expected_duration: duration,
        video_encoder,
        quality,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::tests::{fake_caps, full_caps};
    use crate::probe::{AudioInfo, VideoInfo};

    fn video_stream(index: usize, codec: &str) -> StreamInfo {
        StreamInfo {
            index,
            kind: StreamKind::Video,
            codec: codec.into(),
            codec_long: None,
            profile: None,
            language: None,
            title: None,
            default: true,
            forced: false,
            attached_pic: false,
            bit_rate: None,
            video: Some(VideoInfo {
                width: 1920,
                height: 1080,
                fps: Some(24.0),
                pix_fmt: Some("yuv420p".into()),
                bit_depth: 8,
                color_primaries: None,
                color_transfer: None,
                color_space: None,
                color_range: None,
                hdr: None,
                dolby_vision: false,
                rotation: 0,
                mastering_display: None,
                content_light: None,
            }),
            audio: None,
        }
    }

    fn audio_stream(index: usize, codec: &str, channels: u32) -> StreamInfo {
        StreamInfo {
            kind: StreamKind::Audio,
            codec: codec.into(),
            video: None,
            audio: Some(AudioInfo {
                channels,
                channel_layout: None,
                sample_rate: Some(48000),
            }),
            ..video_stream(index, codec)
        }
    }

    fn sub_stream(index: usize, codec: &str) -> StreamInfo {
        StreamInfo {
            kind: StreamKind::Subtitle,
            codec: codec.into(),
            video: None,
            ..video_stream(index, codec)
        }
    }

    fn media(path: &str, container: &str, streams: Vec<StreamInfo>) -> MediaInfo {
        MediaInfo {
            path: path.into(),
            container: container.into(),
            container_long: String::new(),
            duration_secs: 60.0,
            size_bytes: 1000,
            bit_rate: None,
            start_time_secs: 0.0,
            streams,
        }
    }

    fn compress_opts() -> CompressOptions {
        CompressOptions {
            quality: Quality::High,
            codec: Codec::Av1,
            gpu: false,
            audio_bitrate: None,
            container: None,
        }
    }

    fn actions(plan: &Plan) -> Vec<String> {
        plan.streams
            .iter()
            .map(|s| match &s.action {
                Action::Copy => "copy".to_owned(),
                Action::Transcode(e) => format!("transcode:{}", e.encoder),
                Action::Drop { .. } => "drop".to_owned(),
            })
            .collect()
    }

    #[test]
    fn compress_recodifica_video_e_copia_audio_e_legendas() {
        let info = media(
            "a.mkv",
            "matroska",
            vec![
                video_stream(0, "h264"),
                audio_stream(1, "aac", 2),
                sub_stream(2, "subrip"),
            ],
        );
        let plan = plan_compress(&info, &full_caps(), &compress_opts()).unwrap();
        assert_eq!(actions(&plan), ["transcode:libsvtav1", "copy", "copy"]);
        assert_eq!(plan.ext, "mkv");
        assert_eq!(plan.quality, Some(Quality::High));
        let Action::Transcode(e) = &plan.streams[0].action else {
            panic!()
        };
        assert!(
            e.options
                .contains(&("fps_mode".into(), "passthrough".into()))
        );
    }

    #[test]
    fn compress_avi_vira_mkv() {
        let info = media(
            "antigo.avi",
            "avi",
            vec![video_stream(0, "mpeg4"), audio_stream(1, "mp3", 2)],
        );
        let plan = plan_compress(&info, &full_caps(), &compress_opts()).unwrap();
        assert_eq!(plan.ext, "mkv");
        assert_eq!(plan.muxer, Some("matroska"));
        assert_eq!(actions(&plan), ["transcode:libsvtav1", "copy"]);
    }

    #[test]
    fn compress_webm_com_hevc_vira_mkv_e_mp4_mantem() {
        let webm = media("a.webm", "matroska,webm", vec![video_stream(0, "vp9")]);
        let opts = CompressOptions {
            codec: Codec::Hevc,
            ..compress_opts()
        };
        assert_eq!(compress_container(&webm, &opts).unwrap(), Container::Mkv);
        let mp4 = media("a.mp4", "mp4", vec![video_stream(0, "h264")]);
        assert_eq!(compress_container(&mp4, &opts).unwrap(), Container::Mp4);
        let plan = plan_compress(&mp4, &full_caps(), &opts).unwrap();
        assert!(
            plan.output_options
                .windows(2)
                .any(|w| w == ["-tag:0", "hvc1"])
        );
        assert!(
            plan.output_options
                .windows(2)
                .any(|w| w == ["-movflags", "+faststart"])
        );
    }

    #[test]
    fn compress_container_explicito_incompativel_e_erro_de_uso() {
        let mp4 = media("a.mp4", "mp4", vec![video_stream(0, "h264")]);
        let opts = CompressOptions {
            codec: Codec::Hevc,
            container: Some(Container::Webm),
            ..compress_opts()
        };
        assert_eq!(
            plan_compress(&mp4, &full_caps(), &opts)
                .unwrap_err()
                .exit_code(),
            2
        );
    }

    #[test]
    fn compress_audio_bitrate_recodifica_para_opus_ou_aac_no_mov() {
        let info = media(
            "a.mp4",
            "mp4",
            vec![video_stream(0, "h264"), audio_stream(1, "aac", 2)],
        );
        let opts = CompressOptions {
            audio_bitrate: Some(96),
            ..compress_opts()
        };
        let plan = plan_compress(&info, &full_caps(), &opts).unwrap();
        let Action::Transcode(e) = &plan.streams[1].action else {
            panic!()
        };
        assert_eq!(e.encoder, "libopus");
        assert_eq!(e.options, [("b".to_owned(), "96k".to_owned())]);
        let mov = media(
            "a.mov",
            "mov",
            vec![video_stream(0, "h264"), audio_stream(1, "aac", 2)],
        );
        let plan = plan_compress(&mov, &full_caps(), &opts).unwrap();
        assert_eq!(actions(&plan)[1], "transcode:aac");
    }

    #[test]
    fn compress_hdr_com_h264_e_recusado() {
        let mut v = video_stream(0, "hevc");
        v.video.as_mut().unwrap().hdr = Some(crate::probe::HdrKind::Hdr10);
        let info = media("a.mkv", "matroska", vec![v]);
        let opts = CompressOptions {
            codec: Codec::H264,
            ..compress_opts()
        };
        let err = plan_compress(&info, &full_caps(), &opts).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("--codec av1"));
    }

    #[test]
    fn compress_gpu_usa_nvenc() {
        let info = media("a.mp4", "mp4", vec![video_stream(0, "h264")]);
        let opts = CompressOptions {
            gpu: true,
            ..compress_opts()
        };
        let plan = plan_compress(&info, &full_caps(), &opts).unwrap();
        assert_eq!(plan.video_encoder.as_deref(), Some("av1_nvenc"));
    }

    #[test]
    fn convert_mkv_h264_aac_para_mp4_so_copia() {
        let info = media(
            "a.mkv",
            "matroska",
            vec![video_stream(0, "h264"), audio_stream(1, "aac", 2)],
        );
        let opts = ConvertOptions {
            target: Container::Mp4,
            audio_track: None,
            compat: false,
        };
        let plan = plan_convert(&info, &full_caps(), &opts).unwrap();
        assert_eq!(actions(&plan), ["copy", "copy"]);
        assert!(!plan.transcodes_anything());
    }

    #[test]
    fn convert_pgs_para_mp4_e_descartado_com_aviso() {
        let info = media(
            "a.mkv",
            "matroska",
            vec![
                video_stream(0, "h264"),
                sub_stream(1, "hdmv_pgs_subtitle"),
                sub_stream(2, "subrip"),
            ],
        );
        let opts = ConvertOptions {
            target: Container::Mp4,
            audio_track: None,
            compat: false,
        };
        let plan = plan_convert(&info, &full_caps(), &opts).unwrap();
        assert_eq!(actions(&plan), ["copy", "drop", "transcode:mov_text"]);
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.contains("#1") && w.contains("hdmv_pgs_subtitle"))
        );
    }

    #[test]
    fn convert_h264_para_webm_recodifica_video_e_audio() {
        let info = media(
            "a.mp4",
            "mp4",
            vec![video_stream(0, "h264"), audio_stream(1, "aac", 2)],
        );
        let opts = ConvertOptions {
            target: Container::Webm,
            audio_track: None,
            compat: false,
        };
        let plan = plan_convert(&info, &full_caps(), &opts).unwrap();
        assert_eq!(actions(&plan), ["transcode:libsvtav1", "transcode:libopus"]);
        assert!(plan.streams[0].note.as_deref().unwrap().contains("webm"));
    }

    #[test]
    fn convert_extrai_faixa_de_audio_escolhida() {
        let info = media(
            "a.mkv",
            "matroska",
            vec![
                video_stream(0, "h264"),
                audio_stream(1, "aac", 2),
                audio_stream(2, "ac3", 6),
            ],
        );
        let opts = ConvertOptions {
            target: Container::M4a,
            audio_track: None,
            compat: false,
        };
        let plan = plan_convert(&info, &full_caps(), &opts).unwrap();
        assert_eq!(plan.streams.len(), 1);
        assert_eq!(
            (plan.streams[0].index, actions(&plan)[0].as_str()),
            (1, "copy")
        );
        let opts = ConvertOptions {
            audio_track: Some(2),
            ..opts
        };
        let plan = plan_convert(&info, &full_caps(), &opts).unwrap();
        assert_eq!(
            (plan.streams[0].index, actions(&plan)[0].as_str()),
            (2, "transcode:aac")
        );
        let Action::Transcode(e) = &plan.streams[0].action else {
            panic!()
        };
        assert_eq!(e.options, [("b".to_owned(), "640k".to_owned())]);
        let opts = ConvertOptions {
            audio_track: Some(3),
            ..opts
        };
        assert_eq!(
            plan_convert(&info, &full_caps(), &opts)
                .unwrap_err()
                .exit_code(),
            2
        );
    }

    #[test]
    fn convert_sem_encoder_de_audio_falha_claramente() {
        let caps = fake_caps(&["libsvtav1"], &[]);
        let info = media("a.mp4", "mp4", vec![audio_stream(0, "aac", 2)]);
        let opts = ConvertOptions {
            target: Container::Opus,
            audio_track: None,
            compat: false,
        };
        let err = plan_convert(&info, &caps, &opts).unwrap_err();
        assert!(err.to_string().contains("libopus"), "{err}");
    }

    #[test]
    fn intervalo_de_corte() {
        let r = |f, t, d| resolve_cut_range(300.0, f, t, d);
        assert_eq!(
            r(Some(90.0), Some(165.5), None).unwrap().0,
            CutRange {
                start: 90.0,
                end: 165.5
            }
        );
        assert_eq!(
            r(Some(90.0), None, Some(30.0)).unwrap().0,
            CutRange {
                start: 90.0,
                end: 120.0
            }
        );
        assert_eq!(
            r(None, None, None).unwrap().0,
            CutRange {
                start: 0.0,
                end: 300.0
            }
        );
        assert_eq!(r(Some(120.0), Some(60.0), None).unwrap_err().exit_code(), 2);
        assert_eq!(r(Some(600.0), None, None).unwrap_err().exit_code(), 2);
        assert!(
            r(Some(600.0), None, None)
                .unwrap_err()
                .to_string()
                .contains("00:05:00.000")
        );
        assert_eq!(r(Some(10.0), Some(10.0), None).unwrap_err().exit_code(), 2);
        assert_eq!(r(Some(10.0), None, Some(0.0)).unwrap_err().exit_code(), 2);
        let (range, warn) = r(Some(10.0), Some(600.0), None).unwrap();
        assert_eq!(range.end, 300.0);
        assert!(warn.unwrap().contains("fim"));
    }

    #[test]
    fn corte_rapido_copia_tudo_e_busca_no_keyframe() {
        let info = media(
            "aula.mp4",
            "mp4",
            vec![video_stream(0, "h264"), audio_stream(1, "aac", 2)],
        );
        let opts = CutOptions {
            range: CutRange {
                start: 90.0,
                end: 165.5,
            },
            exact: false,
            keyframe_start: Some(88.4),
            container: None,
        };
        let plan = plan_cut(&info, &full_caps(), &opts).unwrap();
        assert_eq!(actions(&plan), ["copy", "copy"]);
        assert_eq!(plan.input_options, ["-ss", "88.400500"]);
        assert_eq!(plan.output_options[..2], ["-t", "77.100000"]);
        assert!((plan.expected_duration - 77.1).abs() < 1e-9);
        assert_eq!(plan.ext, "mp4");
    }

    #[test]
    fn corte_exato_recodifica_video_no_mesmo_codec() {
        let info = media(
            "a.mp4",
            "mp4",
            vec![video_stream(0, "hevc"), audio_stream(1, "aac", 2)],
        );
        let opts = CutOptions {
            range: CutRange {
                start: 90.0,
                end: 100.0,
            },
            exact: true,
            keyframe_start: Some(88.0),
            container: None,
        };
        let plan = plan_cut(&info, &full_caps(), &opts).unwrap();
        assert_eq!(actions(&plan), ["transcode:libx265", "copy"]);
        assert_eq!(plan.quality, Some(Quality::VisuallyLossless));
        // seek de entrada no keyframe, seek fino na saída (áudio copiado sem pré-roll)
        assert_eq!(plan.input_options, ["-ss", "88.000500"]);
        assert_eq!(
            plan.output_options[..4],
            ["-ss", "1.999500", "-t", "10.000000"]
        );
        assert!((plan.expected_duration - 10.0).abs() < 1e-9);
    }

    #[test]
    fn corte_de_avi_mantem_extensao_sem_muxer_explicito() {
        let info = media("a.avi", "avi", vec![video_stream(0, "mpeg4")]);
        let opts = CutOptions {
            range: CutRange {
                start: 1.0,
                end: 2.0,
            },
            exact: false,
            keyframe_start: Some(0.0),
            container: None,
        };
        let plan = plan_cut(&info, &full_caps(), &opts).unwrap();
        assert_eq!(plan.ext, "avi");
        assert_eq!(plan.muxer, None);
    }

    #[test]
    fn argumentos_do_ffmpeg() {
        let info = media(
            "in.mkv",
            "matroska",
            vec![
                video_stream(0, "h264"),
                sub_stream(1, "hdmv_pgs_subtitle"),
                audio_stream(2, "aac", 2),
            ],
        );
        let opts = ConvertOptions {
            target: Container::Mp4,
            audio_track: None,
            compat: false,
        };
        let plan = plan_convert(&info, &full_caps(), &opts).unwrap();
        let args: Vec<String> = build_args(&plan, Path::new("out.mp4"))
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        assert_eq!(
            args.join(" "),
            "-i in.mkv -map 0:0 -c:0 copy -map 0:2 -c:1 copy -movflags +faststart -f mp4 out.mp4"
        );
    }
}
