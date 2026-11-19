//! Catálogo de codecs e encoders de vídeo que o nago sabe usar.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    Av1,
    Hevc,
    H264,
}

impl Codec {
    pub const ALL: [Codec; 3] = [Codec::Av1, Codec::Hevc, Codec::H264];

    /// Nome do codec como o ffprobe reporta (`codec_name`).
    pub fn ffprobe_name(self) -> &'static str {
        match self {
            Codec::Av1 => "av1",
            Codec::Hevc => "hevc",
            Codec::H264 => "h264",
        }
    }

    pub fn from_ffprobe(name: &str) -> Option<Codec> {
        Codec::ALL.into_iter().find(|c| c.ffprobe_name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Codec::Av1 => "AV1",
            Codec::Hevc => "HEVC",
            Codec::H264 => "H.264",
        }
    }

    pub fn cli_name(self) -> &'static str {
        match self {
            Codec::Av1 => "av1",
            Codec::Hevc => "hevc",
            Codec::H264 => "h264",
        }
    }

    /// AV1 e HEVC carregam HDR; H.264 na prática não (ver spec `video-compression`).
    pub fn supports_hdr(self) -> bool {
        !matches!(self, Codec::H264)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Software,
    Nvenc,
    Qsv,
    Vaapi,
}

impl Backend {
    /// Ordem de preferência para `--gpu`.
    pub const GPU_PREFERENCE: [Backend; 3] = [Backend::Nvenc, Backend::Qsv, Backend::Vaapi];

    pub fn label(self) -> &'static str {
        match self {
            Backend::Software => "software",
            Backend::Nvenc => "NVENC",
            Backend::Qsv => "QSV",
            Backend::Vaapi => "VAAPI",
        }
    }

    pub fn is_hardware(self) -> bool {
        self != Backend::Software
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Encoder {
    pub name: &'static str,
    pub codec: Codec,
    pub backend: Backend,
}

const fn enc(name: &'static str, codec: Codec, backend: Backend) -> Encoder {
    Encoder {
        name,
        codec,
        backend,
    }
}

pub const VIDEO_ENCODERS: [Encoder; 12] = [
    enc("libsvtav1", Codec::Av1, Backend::Software),
    enc("libx265", Codec::Hevc, Backend::Software),
    enc("libx264", Codec::H264, Backend::Software),
    enc("av1_nvenc", Codec::Av1, Backend::Nvenc),
    enc("hevc_nvenc", Codec::Hevc, Backend::Nvenc),
    enc("h264_nvenc", Codec::H264, Backend::Nvenc),
    enc("av1_qsv", Codec::Av1, Backend::Qsv),
    enc("hevc_qsv", Codec::Hevc, Backend::Qsv),
    enc("h264_qsv", Codec::H264, Backend::Qsv),
    enc("av1_vaapi", Codec::Av1, Backend::Vaapi),
    enc("hevc_vaapi", Codec::Hevc, Backend::Vaapi),
    enc("h264_vaapi", Codec::H264, Backend::Vaapi),
];

impl Encoder {
    pub fn by_name(name: &str) -> Option<Encoder> {
        VIDEO_ENCODERS.into_iter().find(|e| e.name == name)
    }

    pub fn software(codec: Codec) -> Encoder {
        VIDEO_ENCODERS
            .into_iter()
            .find(|e| e.codec == codec && e.backend == Backend::Software)
            .expect("todo codec tem encoder de software")
    }

    pub fn for_backend(codec: Codec, backend: Backend) -> Encoder {
        VIDEO_ENCODERS
            .into_iter()
            .find(|e| e.codec == codec && e.backend == backend)
            .expect("toda combinação codec × backend existe no catálogo")
    }
}

/// Dispositivo VAAPI (pode ser trocado por `NAGO_VAAPI_DEVICE`).
pub fn vaapi_device() -> String {
    std::env::var("NAGO_VAAPI_DEVICE").unwrap_or_else(|_| "/dev/dri/renderD128".to_owned())
}
