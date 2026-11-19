use std::path::PathBuf;

/// Erros do núcleo. Cada variante sabe qual código de saída representa
/// (ver spec `job-execution` › Códigos de saída).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Uso incorreto ou validação de entrada (código 2).
    #[error("{0}")]
    Usage(String),

    /// Caminho de entrada inexistente (código 1).
    #[error("arquivo não encontrado: {}", .0.display())]
    NotFound(PathBuf),

    /// Arquivo existe mas não contém mídia reconhecível (código 1).
    #[error("{} não é uma mídia suportada", .0.display())]
    NotMedia(PathBuf),

    /// ffmpeg/ffprobe ausente ou em versão incompatível (código 1).
    #[error("{0}")]
    Ffmpeg(String),

    /// Recurso necessário não disponível (encoder, libvmaf...) (código 1).
    #[error("{0}")]
    Unsupported(String),

    /// O processo do ffmpeg terminou com erro (código 1).
    #[error("{summary}")]
    FfmpegFailed {
        summary: String,
        log_tail: Vec<String>,
        log_file: Option<PathBuf>,
    },

    /// A compressão não reduziu o tamanho e a saída foi descartada (código 3).
    #[error(
        "a compressão não reduziria o tamanho ({input_size} → {output_size} bytes); \
         o original foi mantido e nenhuma saída foi gravada (use --force para manter mesmo assim)"
    )]
    NotSmaller { input_size: u64, output_size: u64 },

    /// Cancelado por sinal (código 130).
    #[error("operação cancelada")]
    Cancelled,

    #[error("erro de E/S: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

impl Error {
    pub fn exit_code(&self) -> u8 {
        match self {
            Error::Usage(_) => 2,
            Error::NotSmaller { .. } => 3,
            Error::Cancelled => 130,
            _ => 1,
        }
    }

    pub fn usage(msg: impl Into<String>) -> Self {
        Error::Usage(msg.into())
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
