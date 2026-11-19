//! Apoio à escolha visual do corte: índice de keyframes e extração de um frame.
//!
//! Os processos do ffmpeg/ffprobe são criados com `kill_on_drop`: cancelar (descartar)
//! o future encerra o processo, o que permite descartar pedidos obsoletos.

use std::process::Stdio;

use crate::probe::MediaInfo;
use crate::tools::Tools;
use crate::{Error, Result};

/// Instantes dos keyframes do vídeo principal, relativos ao início do arquivo.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyframeIndex {
    times: Vec<f64>,
}

/// Folga para comparar instantes (1 ms).
const EPS: f64 = 0.001;

impl KeyframeIndex {
    pub fn from_times(mut times: Vec<f64>) -> KeyframeIndex {
        times.retain(|t| t.is_finite());
        times.sort_by(f64::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < EPS);
        KeyframeIndex { times }
    }

    pub fn len(&self) -> usize {
        self.times.len()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    pub fn times(&self) -> &[f64] {
        &self.times
    }

    /// Keyframe em ou antes de `t` (onde o corte rápido começaria).
    pub fn at_or_before(&self, t: f64) -> Option<f64> {
        let i = self.times.partition_point(|&k| k <= t + EPS);
        i.checked_sub(1).map(|i| self.times[i])
    }

    /// Keyframe estritamente anterior a `t`.
    pub fn before(&self, t: f64) -> Option<f64> {
        let i = self.times.partition_point(|&k| k < t - EPS);
        i.checked_sub(1).map(|i| self.times[i])
    }

    /// Keyframe estritamente posterior a `t`.
    pub fn after(&self, t: f64) -> Option<f64> {
        let i = self.times.partition_point(|&k| k <= t + EPS);
        self.times.get(i).copied()
    }
}

/// Lista os keyframes do vídeo principal lendo só os pacotes (sem decodificar).
/// `None` se o arquivo não tem vídeo.
pub async fn keyframe_index(tools: &Tools, info: &MediaInfo) -> Result<Option<KeyframeIndex>> {
    let Some(video) = info.primary_video() else {
        return Ok(None);
    };
    let out = tokio::process::Command::new(&tools.ffprobe)
        .args(["-v", "error", "-select_streams"])
        .arg(video.index.to_string())
        .args(["-show_entries", "packet=pts_time,flags", "-of", "csv=p=0"])
        .arg(&info.path)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "ffprobe falhou ao listar keyframes: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(Some(parse_packets_csv(
        &String::from_utf8_lossy(&out.stdout),
        info.start_time_secs,
    )))
}

/// Linhas `pts_time,flags` → keyframes relativos a `start`.
fn parse_packets_csv(text: &str, start: f64) -> KeyframeIndex {
    let times = text
        .lines()
        .filter_map(|line| {
            let (pts, flags) = line.split_once(',')?;
            if !flags.starts_with('K') {
                return None;
            }
            let pts: f64 = pts.trim().parse().ok()?;
            Some((pts - start).max(0.0))
        })
        .collect();
    KeyframeIndex::from_times(times)
}

/// Filtros para a miniatura: corrige pixels não quadrados, mapeia HDR para SDR (se
/// `tonemap`), reduz para caber em `max_w`x`max_h` mantendo a proporção e converte para RGB.
pub fn thumbnail_filter(hdr: bool, tonemap: bool, max_w: u32, max_h: u32) -> String {
    let mut chain = Vec::new();
    if hdr && tonemap {
        chain.push(
            "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=hable:desat=0,\
             zscale=t=bt709:m=bt709:r=tv"
                .to_owned(),
        );
    }
    chain.push("scale=iw*sar:ih,setsar=1".to_owned());
    chain.push(format!(
        "scale={max_w}:{max_h}:force_original_aspect_ratio=decrease"
    ));
    chain.push("format=rgb24".to_owned());
    chain.join(",")
}

/// Extrai o frame do instante `t` (relativo ao início) como PNG que cabe em
/// `max_w`x`max_h`. A rotação do arquivo é aplicada (autorotate do ffmpeg). Com
/// `tonemap` (requer o filtro `zscale`), vídeo HDR é mapeado para SDR.
pub async fn frame_png(
    tools: &Tools,
    info: &MediaInfo,
    t: f64,
    max_w: u32,
    max_h: u32,
    tonemap: bool,
) -> Result<Vec<u8>> {
    let Some(video) = info.primary_video() else {
        return Err(Error::usage("o arquivo não tem vídeo"));
    };
    let hdr = video.video.as_ref().is_some_and(|v| v.hdr.is_some());
    let filter = thumbnail_filter(hdr, tonemap, max_w.max(2), max_h.max(2));
    let out = tokio::process::Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-ss"])
        .arg(format!("{:.3}", t.max(0.0)))
        .arg("-i")
        .arg(&info.path)
        .args(["-map", &format!("0:{}", video.index)])
        .args(["-frames:v", "1", "-vf", &filter])
        .args(["-f", "image2pipe", "-c:v", "png", "-"])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "ffmpeg falhou ao extrair o frame: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    if out.stdout.is_empty() {
        return Err(Error::Other(format!(
            "nenhum frame em {}",
            crate::timefmt::format(t)
        )));
    }
    Ok(out.stdout)
}

/// Largura, altura e profundidade de bits de um PNG (lidas do cabeçalho IHDR).
pub fn png_dimensions(png: &[u8]) -> Option<(u32, u32, u8)> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if png.len() < 25 || !png.starts_with(SIG) || &png[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(png[20..24].try_into().ok()?);
    Some((w, h, png[24]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx() -> KeyframeIndex {
        KeyframeIndex::from_times(vec![4.0, 0.0, 2.0, 6.0, 2.0005])
    }

    #[test]
    fn indice_ordena_e_remove_duplicados() {
        assert_eq!(idx().times(), &[0.0, 2.0, 4.0, 6.0]);
    }

    #[test]
    fn busca_de_vizinhos() {
        let k = idx();
        assert_eq!(k.at_or_before(5.3), Some(4.0));
        assert_eq!(k.at_or_before(4.0), Some(4.0));
        assert_eq!(k.before(4.0), Some(2.0));
        assert_eq!(k.after(5.3), Some(6.0));
        assert_eq!(k.after(4.0), Some(6.0));
        assert_eq!(k.after(6.0), None);
        assert_eq!(k.before(0.0), None);
        assert_eq!(KeyframeIndex::default().at_or_before(1.0), None);
    }

    #[test]
    fn csv_de_pacotes() {
        let csv = "1.400000,K__\n1.433333,___\n3.400000,K_\nN/A,K__\n";
        let k = parse_packets_csv(csv, 1.4);
        assert_eq!(k.len(), 2);
        assert!((k.times()[1] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn filtro_com_e_sem_hdr() {
        let sdr = thumbnail_filter(false, true, 320, 180);
        assert!(!sdr.contains("tonemap"));
        assert!(sdr.contains("scale=320:180:force_original_aspect_ratio=decrease"));
        assert!(thumbnail_filter(true, true, 320, 180).starts_with("zscale"));
        assert!(!thumbnail_filter(true, false, 320, 180).contains("zscale"));
    }

    #[test]
    fn cabecalho_png() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend(320u32.to_be_bytes());
        png.extend(180u32.to_be_bytes());
        png.push(8);
        assert_eq!(png_dimensions(&png), Some((320, 180, 8)));
        assert_eq!(png_dimensions(b"nope"), None);
    }
}
