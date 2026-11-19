//! Containers de saída e matriz de compatibilidade container × codec (design D5).

use serde::Serialize;

use crate::encoders::Codec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    Mp4,
    Mov,
    Mkv,
    Webm,
    M4a,
    Mp3,
    Opus,
    Flac,
}

/// O que fazer com um stream ao gravá-lo num container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compat {
    /// Copiar sem recodificar.
    Copy,
    /// Recodificar com este encoder (áudio/legenda); vídeo usa o encoder de vídeo escolhido.
    Transcode(&'static str),
    /// Não cabe no container; descartar com o motivo.
    Drop(String),
}

const TEXT_SUBS: [&str; 6] = ["subrip", "ass", "ssa", "webvtt", "mov_text", "text"];
const COVER_CODECS: [&str; 3] = ["mjpeg", "png", "bmp"];

impl Container {
    pub const ALL: [Container; 8] = [
        Container::Mp4,
        Container::Mkv,
        Container::Mov,
        Container::Webm,
        Container::M4a,
        Container::Mp3,
        Container::Opus,
        Container::Flac,
    ];

    pub fn parse(name: &str) -> Option<Container> {
        Some(
            match name
                .trim()
                .trim_start_matches('.')
                .to_ascii_lowercase()
                .as_str()
            {
                "mp4" | "m4v" => Container::Mp4,
                "mkv" | "matroska" => Container::Mkv,
                "mov" => Container::Mov,
                "webm" => Container::Webm,
                "m4a" => Container::M4a,
                "mp3" => Container::Mp3,
                "opus" => Container::Opus,
                "flac" => Container::Flac,
                _ => return None,
            },
        )
    }

    /// Container de vídeo correspondente ao arquivo de entrada (`container` do probe + extensão).
    pub fn from_input(probe_container: &str, ext: Option<&str>) -> Option<Container> {
        let c = ext
            .and_then(Container::parse)
            .or_else(|| Container::parse(probe_container))?;
        (!c.is_audio_only()).then_some(c)
    }

    pub fn names() -> String {
        Container::ALL.map(|c| c.ext()).join(", ")
    }

    pub fn ext(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mov => "mov",
            Container::Mkv => "mkv",
            Container::Webm => "webm",
            Container::M4a => "m4a",
            Container::Mp3 => "mp3",
            Container::Opus => "opus",
            Container::Flac => "flac",
        }
    }

    /// Nome do muxer do ffmpeg (`-f`).
    pub fn muxer(self) -> &'static str {
        match self {
            Container::Mp4 => "mp4",
            Container::Mov => "mov",
            Container::Mkv => "matroska",
            Container::Webm => "webm",
            Container::M4a => "ipod",
            Container::Mp3 => "mp3",
            Container::Opus => "opus",
            Container::Flac => "flac",
        }
    }

    pub fn is_audio_only(self) -> bool {
        matches!(
            self,
            Container::M4a | Container::Mp3 | Container::Opus | Container::Flac
        )
    }

    /// Família MP4/QuickTime (moov atom, faststart, mov_text).
    pub fn is_isobmff(self) -> bool {
        matches!(self, Container::Mp4 | Container::Mov | Container::M4a)
    }

    /// O container aceita vídeo neste codec de saída?
    pub fn supports_video_codec(self, codec: Codec) -> bool {
        self.video_compat(codec.ffprobe_name())
    }

    fn video_compat(self, codec: &str) -> bool {
        match self {
            Container::Mkv => true,
            Container::Mp4 => matches!(codec, "h264" | "hevc" | "av1"),
            Container::Mov => {
                matches!(
                    codec,
                    "h264" | "hevc" | "av1" | "prores" | "mjpeg" | "dnxhd" | "mpeg4"
                )
            }
            Container::Webm => matches!(codec, "av1" | "vp9" | "vp8"),
            _ => false,
        }
    }

    /// Decisão para um stream de vídeo "de verdade" (não capa).
    /// `Transcode("")` significa: recodificar com o encoder de vídeo escolhido.
    pub fn video(self, codec: &str) -> Compat {
        if self.is_audio_only() {
            Compat::Drop("formato somente de áudio".into())
        } else if self.video_compat(codec) {
            Compat::Copy
        } else {
            Compat::Transcode("")
        }
    }

    /// Capa/miniatura embutida.
    pub fn cover(self, codec: &str) -> Compat {
        match self {
            Container::Mkv => Compat::Copy,
            Container::Mp4 | Container::Mov if COVER_CODECS.contains(&codec) => Compat::Copy,
            _ => Compat::Drop(format!("capa {codec} não é suportada em {}", self.ext())),
        }
    }

    /// Decisão para áudio. `compat` troca Opus/FLAC por AAC em mp4 (players antigos).
    pub fn audio(self, codec: &str, compat: bool) -> Compat {
        let pcm = codec.starts_with("pcm_");
        let ok = match self {
            Container::Mkv => true,
            Container::Mp4 => {
                matches!(codec, "aac" | "mp3" | "ac3" | "eac3" | "alac")
                    || (!compat && matches!(codec, "opus" | "flac"))
            }
            Container::Mov => matches!(codec, "aac" | "mp3" | "ac3" | "eac3" | "alac") || pcm,
            Container::Webm => matches!(codec, "opus" | "vorbis"),
            Container::M4a => matches!(codec, "aac" | "alac"),
            Container::Mp3 => codec == "mp3",
            Container::Opus => codec == "opus",
            Container::Flac => codec == "flac",
        };
        if ok {
            Compat::Copy
        } else {
            Compat::Transcode(self.audio_fallback())
        }
    }

    /// Encoder de áudio usado quando o codec original não cabe.
    pub fn audio_fallback(self) -> &'static str {
        match self {
            Container::Webm | Container::Opus => "libopus",
            Container::Mp3 => "libmp3lame",
            Container::Flac => "flac",
            Container::Mkv => "libopus",
            _ => "aac",
        }
    }

    /// Decisão para legendas.
    pub fn subtitle(self, codec: &str) -> Compat {
        let text = TEXT_SUBS.contains(&codec);
        match self {
            Container::Mp4 | Container::Mov => match codec {
                "mov_text" => Compat::Copy,
                _ if text => Compat::Transcode("mov_text"),
                _ => Compat::Drop(format!(
                    "legenda de imagem ({codec}) não é suportada em {}",
                    self.ext()
                )),
            },
            Container::Mkv => match codec {
                "mov_text" => Compat::Transcode("srt"),
                _ => Compat::Copy,
            },
            Container::Webm => match codec {
                "webvtt" => Compat::Copy,
                _ if text => Compat::Transcode("webvtt"),
                _ => Compat::Drop(format!("legenda {codec} não é suportada em webm")),
            },
            _ => Compat::Drop("formato somente de áudio".into()),
        }
    }

    /// Anexos (fontes etc.) só existem no Matroska.
    pub fn attachment(self) -> Compat {
        match self {
            Container::Mkv => Compat::Copy,
            _ => Compat::Drop(format!("anexos não são suportados em {}", self.ext())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Compat::*;

    #[test]
    fn parse_de_nomes() {
        assert_eq!(Container::parse("MP4"), Some(Container::Mp4));
        assert_eq!(Container::parse(".mkv"), Some(Container::Mkv));
        assert_eq!(Container::parse("matroska"), Some(Container::Mkv));
        assert_eq!(Container::parse("avi"), None);
        assert_eq!(
            Container::from_input("matroska", Some("mkv")),
            Some(Container::Mkv)
        );
        assert_eq!(Container::from_input("avi", Some("avi")), None);
        assert_eq!(Container::from_input("mp3", Some("mp3")), None);
    }

    #[test]
    fn matriz_de_video() {
        let cases = [
            (Container::Mp4, "h264", Copy),
            (Container::Mp4, "hevc", Copy),
            (Container::Mp4, "av1", Copy),
            (Container::Mp4, "mpeg2video", Transcode("")),
            (Container::Mp4, "prores", Transcode("")),
            (Container::Mov, "prores", Copy),
            (Container::Mov, "h264", Copy),
            (Container::Mkv, "mpeg2video", Copy),
            (Container::Mkv, "vc1", Copy),
            (Container::Webm, "h264", Transcode("")),
            (Container::Webm, "vp9", Copy),
            (Container::Webm, "av1", Copy),
        ];
        for (c, codec, expected) in cases {
            assert_eq!(c.video(codec), expected, "{c:?} {codec}");
        }
        assert!(matches!(Container::M4a.video("h264"), Drop(_)));
    }

    #[test]
    fn matriz_de_audio() {
        let cases = [
            (Container::Mp4, "aac", false, Copy),
            (Container::Mp4, "ac3", false, Copy),
            (Container::Mp4, "opus", false, Copy),
            (Container::Mp4, "opus", true, Transcode("aac")),
            (Container::Mp4, "flac", true, Transcode("aac")),
            (Container::Mp4, "pcm_s16le", false, Transcode("aac")),
            (Container::Mp4, "dts", false, Transcode("aac")),
            (Container::Mov, "pcm_s16le", false, Copy),
            (Container::Mov, "opus", false, Transcode("aac")),
            (Container::Mkv, "dts", false, Copy),
            (Container::Mkv, "pcm_s24le", false, Copy),
            (Container::Webm, "aac", false, Transcode("libopus")),
            (Container::Webm, "vorbis", false, Copy),
            (Container::M4a, "aac", false, Copy),
            (Container::M4a, "opus", false, Transcode("aac")),
            (Container::Mp3, "mp3", false, Copy),
            (Container::Mp3, "aac", false, Transcode("libmp3lame")),
            (Container::Opus, "aac", false, Transcode("libopus")),
            (Container::Flac, "pcm_s16le", false, Transcode("flac")),
            (Container::Flac, "flac", false, Copy),
        ];
        for (c, codec, compat, expected) in cases {
            assert_eq!(
                c.audio(codec, compat),
                expected,
                "{c:?} {codec} compat={compat}"
            );
        }
    }

    #[test]
    fn matriz_de_legendas() {
        let cases = [
            (Container::Mp4, "subrip", Some(Transcode("mov_text"))),
            (Container::Mp4, "ass", Some(Transcode("mov_text"))),
            (Container::Mp4, "mov_text", Some(Copy)),
            (Container::Mp4, "hdmv_pgs_subtitle", None),
            (Container::Mov, "dvd_subtitle", None),
            (Container::Mkv, "subrip", Some(Copy)),
            (Container::Mkv, "hdmv_pgs_subtitle", Some(Copy)),
            (Container::Mkv, "mov_text", Some(Transcode("srt"))),
            (Container::Webm, "subrip", Some(Transcode("webvtt"))),
            (Container::Webm, "webvtt", Some(Copy)),
            (Container::Webm, "hdmv_pgs_subtitle", None),
        ];
        for (c, codec, expected) in cases {
            let got = c.subtitle(codec);
            match expected {
                Some(e) => assert_eq!(got, e, "{c:?} {codec}"),
                None => assert!(
                    matches!(got, Drop(ref r) if r.contains(codec)),
                    "{c:?} {codec}: {got:?}"
                ),
            }
        }
    }

    #[test]
    fn capas_e_anexos() {
        assert_eq!(Container::Mp4.cover("mjpeg"), Copy);
        assert_eq!(Container::Mkv.cover("png"), Copy);
        assert!(matches!(Container::Webm.cover("mjpeg"), Drop(_)));
        assert_eq!(Container::Mkv.attachment(), Copy);
        assert!(matches!(Container::Mp4.attachment(), Drop(_)));
    }

    #[test]
    fn suporte_a_codec_de_saida() {
        assert!(Container::Mp4.supports_video_codec(Codec::Av1));
        assert!(Container::Mkv.supports_video_codec(Codec::Hevc));
        assert!(!Container::Webm.supports_video_codec(Codec::Hevc));
        assert!(!Container::Webm.supports_video_codec(Codec::H264));
        assert!(Container::Webm.supports_video_codec(Codec::Av1));
    }
}
