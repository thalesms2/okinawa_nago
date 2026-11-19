//! Leitura e normalização de metadados via `ffprobe` (spec `media-probe`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tools::Tools;
use crate::{Error, Result};

/// Metadados normalizados de um arquivo. Este é o esquema estável emitido por `nago info --json`.
#[derive(Debug, Clone, Serialize)]
pub struct MediaInfo {
    pub path: PathBuf,
    /// Nome curto do container (`mp4`, `matroska`, `mov`, `avi`...).
    pub container: String,
    pub container_long: String,
    pub duration_secs: f64,
    pub size_bytes: u64,
    pub bit_rate: Option<u64>,
    /// Timestamp inicial do arquivo (os instantes do usuário são relativos a ele).
    pub start_time_secs: f64,
    pub streams: Vec<StreamInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    Data,
    Attachment,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamInfo {
    pub index: usize,
    pub kind: StreamKind,
    pub codec: String,
    pub codec_long: Option<String>,
    pub profile: Option<String>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub default: bool,
    pub forced: bool,
    /// Capa/miniatura embutida (stream de vídeo que não é vídeo de verdade).
    pub attached_pic: bool,
    pub bit_rate: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video: Option<VideoInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<AudioInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
    pub pix_fmt: Option<String>,
    pub bit_depth: u8,
    pub color_primaries: Option<String>,
    pub color_transfer: Option<String>,
    pub color_space: Option<String>,
    pub color_range: Option<String>,
    /// `"HDR10"`, `"HLG"` ou `null` para SDR.
    pub hdr: Option<HdrKind>,
    pub dolby_vision: bool,
    /// Rotação em graus indicada pela display matrix (0, 90, -90, 180).
    pub rotation: i32,
    pub mastering_display: Option<MasteringDisplay>,
    pub content_light: Option<ContentLight>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioInfo {
    pub channels: u32,
    pub channel_layout: Option<String>,
    pub sample_rate: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum HdrKind {
    #[serde(rename = "HDR10")]
    Hdr10,
    #[serde(rename = "HLG")]
    Hlg,
}

/// Cromaticidades (0..1) e luminâncias (cd/m²) do mastering display.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct MasteringDisplay {
    pub red: (f64, f64),
    pub green: (f64, f64),
    pub blue: (f64, f64),
    pub white_point: (f64, f64),
    pub min_luminance: f64,
    pub max_luminance: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ContentLight {
    pub max_cll: u32,
    pub max_fall: u32,
}

impl MediaInfo {
    /// Stream de vídeo principal (o primeiro que não é capa).
    pub fn primary_video(&self) -> Option<&StreamInfo> {
        self.streams
            .iter()
            .find(|s| s.kind == StreamKind::Video && !s.attached_pic)
    }

    pub fn audio_streams(&self) -> impl Iterator<Item = &StreamInfo> {
        self.streams.iter().filter(|s| s.kind == StreamKind::Audio)
    }

    /// Extensão do arquivo de entrada em minúsculas.
    pub fn extension(&self) -> Option<String> {
        self.path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
    }
}

impl StreamInfo {
    pub fn is_hdr(&self) -> bool {
        self.video.as_ref().is_some_and(|v| v.hdr.is_some())
    }
}

/// Inspeciona um arquivo.
pub async fn probe(tools: &Tools, path: &Path) -> Result<MediaInfo> {
    if !path.exists() {
        return Err(Error::NotFound(path.to_path_buf()));
    }
    if path.is_dir() {
        return Err(Error::NotMedia(path.to_path_buf()));
    }
    let out = tokio::process::Command::new(&tools.ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .await?;
    if !out.status.success() {
        return Err(Error::NotMedia(path.to_path_buf()));
    }
    let json: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| Error::Other(format!("saída inesperada do ffprobe: {e}")))?;
    let mut info = parse_probe(path, &json)?;

    // O primeiro frame do vídeo principal traz cor e metadados HDR que nem todo
    // container repete no nível do stream.
    if let Some(idx) = info.primary_video().map(|s| s.index) {
        if let Ok(frame) = first_frame(tools, path, idx).await {
            let stream = info.streams.iter_mut().find(|s| s.index == idx).unwrap();
            enrich_with_frame(stream.video.as_mut().unwrap(), &frame);
        }
    }
    Ok(info)
}

/// Converte o JSON do ffprobe (`-show_format -show_streams`) em [`MediaInfo`].
pub fn parse_probe(path: &Path, json: &Value) -> Result<MediaInfo> {
    let not_media = || Error::NotMedia(path.to_path_buf());
    let format = json.get("format").ok_or_else(not_media)?;
    let format_name = str_of(format, "format_name").unwrap_or_default();
    if ["tty", "image2", "lrc", "srt", "ass", "webvtt"].contains(&format_name.as_str())
        || format_name.ends_with("_pipe")
    {
        return Err(not_media());
    }

    let streams: Vec<StreamInfo> = json
        .get("streams")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(parse_stream).collect())
        .unwrap_or_default();
    let has_av = streams
        .iter()
        .any(|s| (s.kind == StreamKind::Video && !s.attached_pic) || s.kind == StreamKind::Audio);
    if !has_av {
        return Err(not_media());
    }

    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    let container = short_container(&format_name, ext.as_deref());

    Ok(MediaInfo {
        path: path.to_path_buf(),
        container,
        container_long: str_of(format, "format_long_name").unwrap_or_default(),
        duration_secs: num_of(format, "duration").unwrap_or(0.0),
        size_bytes: num_of(format, "size")
            .map(|v| v as u64)
            .unwrap_or_else(|| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)),
        bit_rate: num_of(format, "bit_rate").map(|v| v as u64),
        start_time_secs: num_of(format, "start_time").unwrap_or(0.0),
        streams,
    })
}

/// `mov,mp4,m4a,3gp,3g2,mj2` → `mp4` (ou o membro que bate com a extensão).
fn short_container(format_name: &str, ext: Option<&str>) -> String {
    let names: Vec<&str> = format_name.split(',').collect();
    if let Some(ext) = ext {
        let ext = match ext {
            "mkv" | "mka" => "matroska",
            other => other,
        };
        if names.contains(&ext) {
            return ext.to_owned();
        }
    }
    names.first().copied().unwrap_or("desconhecido").to_owned()
}

fn parse_stream(s: &Value) -> Option<StreamInfo> {
    let kind = match s.get("codec_type")?.as_str()? {
        "video" => StreamKind::Video,
        "audio" => StreamKind::Audio,
        "subtitle" => StreamKind::Subtitle,
        "attachment" => StreamKind::Attachment,
        _ => StreamKind::Data,
    };
    let disposition = |k: &str| {
        s.get("disposition")
            .and_then(|d| d.get(k))
            .and_then(Value::as_i64)
            == Some(1)
    };
    let tag = |k: &str| {
        s.get("tags")
            .and_then(|t| t.get(k).or_else(|| t.get(k.to_uppercase())))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let video = (kind == StreamKind::Video).then(|| parse_video(s));
    let audio = (kind == StreamKind::Audio).then(|| AudioInfo {
        channels: s.get("channels").and_then(Value::as_u64).unwrap_or(0) as u32,
        channel_layout: str_of(s, "channel_layout"),
        sample_rate: num_of(s, "sample_rate").map(|v| v as u32),
    });
    Some(StreamInfo {
        index: s.get("index")?.as_u64()? as usize,
        kind,
        codec: str_of(s, "codec_name").unwrap_or_else(|| "desconhecido".into()),
        codec_long: str_of(s, "codec_long_name"),
        profile: str_of(s, "profile"),
        language: tag("language").filter(|l| l != "und"),
        title: tag("title"),
        default: disposition("default"),
        forced: disposition("forced"),
        attached_pic: disposition("attached_pic"),
        bit_rate: num_of(s, "bit_rate").map(|v| v as u64),
        video,
        audio,
    })
}

fn parse_video(s: &Value) -> VideoInfo {
    let pix_fmt = str_of(s, "pix_fmt");
    let bit_depth = num_of(s, "bits_per_raw_sample")
        .map(|v| v as u8)
        .unwrap_or_else(|| depth_from_pix_fmt(pix_fmt.as_deref()));
    let side = s.get("side_data_list").and_then(Value::as_array);
    let rotation = side
        .into_iter()
        .flatten()
        .find_map(|d| d.get("rotation").and_then(Value::as_f64))
        .map(|r| r.round() as i32)
        .unwrap_or(0);
    let dolby_vision = side
        .into_iter()
        .flatten()
        .any(|d| str_of(d, "side_data_type").is_some_and(|t| t.contains("DOVI")));
    let mut v = VideoInfo {
        width: s.get("width").and_then(Value::as_u64).unwrap_or(0) as u32,
        height: s.get("height").and_then(Value::as_u64).unwrap_or(0) as u32,
        fps: parse_rate(s, "avg_frame_rate").or_else(|| parse_rate(s, "r_frame_rate")),
        pix_fmt,
        bit_depth,
        color_primaries: known(str_of(s, "color_primaries")),
        color_transfer: known(str_of(s, "color_transfer")),
        color_space: known(str_of(s, "color_space")),
        color_range: known(str_of(s, "color_range")),
        hdr: None,
        dolby_vision,
        rotation,
        mastering_display: None,
        content_light: None,
    };
    if let Some(list) = side {
        apply_hdr_side_data(&mut v, list);
    }
    v.hdr = hdr_kind(v.color_transfer.as_deref());
    v
}

fn depth_from_pix_fmt(pix_fmt: Option<&str>) -> u8 {
    let p = pix_fmt.unwrap_or("");
    if p.contains("16") {
        16
    } else if p.contains("12") {
        12
    } else if p.contains("10") {
        10
    } else {
        8
    }
}

fn hdr_kind(transfer: Option<&str>) -> Option<HdrKind> {
    match transfer? {
        "smpte2084" => Some(HdrKind::Hdr10),
        "arib-std-b67" => Some(HdrKind::Hlg),
        _ => None,
    }
}

/// Completa cor/HDR com os dados do primeiro frame (JSON de `-show_frames`).
fn enrich_with_frame(v: &mut VideoInfo, frame: &Value) {
    if v.color_transfer.is_none() {
        v.color_transfer = known(str_of(frame, "color_transfer"));
    }
    if v.color_primaries.is_none() {
        v.color_primaries = known(str_of(frame, "color_primaries"));
    }
    if v.color_space.is_none() {
        v.color_space = known(str_of(frame, "color_space"));
    }
    if v.color_range.is_none() {
        v.color_range = known(str_of(frame, "color_range"));
    }
    if let Some(list) = frame.get("side_data_list").and_then(Value::as_array) {
        apply_hdr_side_data(v, list);
    }
    v.hdr = hdr_kind(v.color_transfer.as_deref());
}

fn apply_hdr_side_data(v: &mut VideoInfo, list: &[Value]) {
    for d in list {
        match str_of(d, "side_data_type").as_deref() {
            Some("Mastering display metadata") if v.mastering_display.is_none() => {
                let r = |k: &str| d.get(k).and_then(Value::as_str).and_then(parse_ratio);
                if let (
                    Some(rx),
                    Some(ry),
                    Some(gx),
                    Some(gy),
                    Some(bx),
                    Some(by),
                    Some(wx),
                    Some(wy),
                    Some(minl),
                    Some(maxl),
                ) = (
                    r("red_x"),
                    r("red_y"),
                    r("green_x"),
                    r("green_y"),
                    r("blue_x"),
                    r("blue_y"),
                    r("white_point_x"),
                    r("white_point_y"),
                    r("min_luminance"),
                    r("max_luminance"),
                ) {
                    v.mastering_display = Some(MasteringDisplay {
                        red: (rx, ry),
                        green: (gx, gy),
                        blue: (bx, by),
                        white_point: (wx, wy),
                        min_luminance: minl,
                        max_luminance: maxl,
                    });
                }
            }
            Some("Content light level metadata") if v.content_light.is_none() => {
                let n = |k: &str| d.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
                v.content_light = Some(ContentLight {
                    max_cll: n("max_content"),
                    max_fall: n("max_average"),
                });
            }
            Some(t) if t.contains("DOVI") => v.dolby_vision = true,
            _ => {}
        }
    }
}

async fn first_frame(tools: &Tools, path: &Path, stream_index: usize) -> Result<Value> {
    let out = tokio::process::Command::new(&tools.ffprobe)
        .args(["-v", "error", "-select_streams"])
        .arg(stream_index.to_string())
        .args(["-read_intervals", "%+#1", "-show_frames", "-show_entries"])
        .arg("frame=color_transfer,color_primaries,color_space,color_range,side_data_list")
        .args(["-print_format", "json"])
        .arg(path)
        .output()
        .await?;
    let json: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| Error::Other(format!("saída inesperada do ffprobe: {e}")))?;
    json.get("frames")
        .and_then(|f| f.get(0))
        .cloned()
        .ok_or_else(|| Error::Other("nenhum frame lido".into()))
}

/// Instante (relativo ao início do arquivo) do keyframe do vídeo principal
/// anterior ou igual a `t`. `None` se o arquivo não tem vídeo.
pub async fn keyframe_at_or_before(tools: &Tools, info: &MediaInfo, t: f64) -> Result<Option<f64>> {
    let Some(video) = info.primary_video() else {
        return Ok(None);
    };
    let start = info.start_time_secs;
    let target = t + start;
    // Janela curta primeiro (instantâneo mesmo em arquivos enormes); se o GOP for
    // maior que a janela, procura desde o início.
    for window_start in [(target - 20.0).max(start), start] {
        let interval = format!("{window_start:.6}%{:.6}", target + 1.0);
        let out = tokio::process::Command::new(&tools.ffprobe)
            .args(["-v", "error", "-select_streams"])
            .arg(video.index.to_string())
            .args(["-read_intervals", &interval])
            .args([
                "-show_entries",
                "packet=pts_time,flags",
                "-print_format",
                "json",
            ])
            .arg(&info.path)
            .output()
            .await?;
        let json: Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| Error::Other(format!("saída inesperada do ffprobe: {e}")))?;
        let best = best_keyframe(&json, target);
        if let Some(kf) = best {
            return Ok(Some((kf - start).max(0.0)));
        }
    }
    Ok(Some(0.0))
}

/// Maior `pts_time` de pacote keyframe que seja <= `target` (com folga de 1 ms).
fn best_keyframe(packets_json: &Value, target: f64) -> Option<f64> {
    packets_json
        .get("packets")?
        .as_array()?
        .iter()
        .filter(|p| str_of(p, "flags").is_some_and(|f| f.starts_with('K')))
        .filter_map(|p| num_of(p, "pts_time"))
        .filter(|&pts| pts <= target + 0.001)
        .max_by(f64::total_cmp)
}

fn known(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.is_empty() && s != "unknown" && s != "reserved")
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Números do ffprobe chegam como string (`"6.021000"`) ou número; `"N/A"` vira `None`.
fn num_of(v: &Value, key: &str) -> Option<f64> {
    match v.get(key)? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
    .filter(|x: &f64| x.is_finite())
}

fn parse_rate(v: &Value, key: &str) -> Option<f64> {
    parse_ratio(v.get(key)?.as_str()?).filter(|r| *r > 0.0)
}

fn parse_ratio(s: &str) -> Option<f64> {
    match s.split_once('/') {
        Some((n, d)) => {
            let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
            (d != 0.0).then(|| n / d)
        }
        None => s.parse().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn container_curto() {
        assert_eq!(
            short_container("mov,mp4,m4a,3gp,3g2,mj2", Some("mp4")),
            "mp4"
        );
        assert_eq!(
            short_container("mov,mp4,m4a,3gp,3g2,mj2", Some("mov")),
            "mov"
        );
        assert_eq!(
            short_container("mov,mp4,m4a,3gp,3g2,mj2", Some("xyz")),
            "mov"
        );
        assert_eq!(short_container("matroska,webm", Some("mkv")), "matroska");
        assert_eq!(short_container("matroska,webm", Some("webm")), "webm");
        assert_eq!(short_container("avi", Some("avi")), "avi");
    }

    #[test]
    fn hdr_por_transferencia() {
        assert_eq!(hdr_kind(Some("smpte2084")), Some(HdrKind::Hdr10));
        assert_eq!(hdr_kind(Some("arib-std-b67")), Some(HdrKind::Hlg));
        assert_eq!(hdr_kind(Some("bt709")), None);
        assert_eq!(hdr_kind(None), None);
    }

    #[test]
    fn profundidade_pelo_pix_fmt() {
        assert_eq!(depth_from_pix_fmt(Some("yuv420p")), 8);
        assert_eq!(depth_from_pix_fmt(Some("yuv420p10le")), 10);
        assert_eq!(depth_from_pix_fmt(Some("p010le")), 10);
        assert_eq!(depth_from_pix_fmt(Some("yuv444p12le")), 12);
    }

    #[test]
    fn texto_nao_e_midia() {
        let j = json!({"format": {"format_name": "tty"}, "streams": [{"index":0,"codec_type":"video","codec_name":"ansi"}]});
        assert!(matches!(
            parse_probe(Path::new("a.txt"), &j),
            Err(Error::NotMedia(_))
        ));
    }

    #[test]
    fn so_legenda_nao_e_midia() {
        let j = json!({"format": {"format_name": "matroska,webm"}, "streams": [{"index":0,"codec_type":"subtitle","codec_name":"subrip"}]});
        assert!(matches!(
            parse_probe(Path::new("a.mkv"), &j),
            Err(Error::NotMedia(_))
        ));
    }

    #[test]
    fn keyframe_mais_proximo() {
        let j = json!({"packets": [
            {"pts_time": "2.000000", "flags": "K__"},
            {"pts_time": "2.033333", "flags": "___"},
            {"pts_time": "4.000000", "flags": "K__"},
            {"pts_time": "5.300000", "flags": "___"},
            {"pts_time": "6.000000", "flags": "K__"}
        ]});
        assert_eq!(best_keyframe(&j, 5.3), Some(4.0));
        assert_eq!(best_keyframe(&j, 4.0), Some(4.0));
        assert_eq!(best_keyframe(&j, 1.0), None);
    }
}
