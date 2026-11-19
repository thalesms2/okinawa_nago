use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "nago",
    version,
    about = "Cortar, converter e comprimir vídeos sem perda perceptível de qualidade",
    propagate_version = true
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalOpts,

    /// Sem subcomando, num terminal, abre o modo interativo (TUI).
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Args, Debug, Clone)]
pub struct GlobalOpts {
    /// Emite o resultado como JSON em stdout (nada mais é escrito em stdout)
    #[arg(long, global = true)]
    pub json: bool,

    /// Mostra logs detalhados e salva o log completo do ffmpeg
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Permite sobrescrever um arquivo existente informado em -o (nunca a entrada)
    #[arg(long, global = true)]
    pub overwrite: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Mostra os metadados de um arquivo de mídia
    Info(InfoArgs),
    /// Extrai um trecho do vídeo (instantâneo por padrão, preciso ao frame com --exact)
    Cut(CutArgs),
    /// Troca o container/formato, copiando os streams sempre que possível
    Convert(ConvertArgs),
    /// Recodifica para reduzir o tamanho mantendo a qualidade perceptível
    Compress(CompressArgs),
    /// Mostra o ffmpeg encontrado, os encoders disponíveis e se cada um funciona
    Doctor,
    /// Abre o modo interativo (TUI), opcionalmente com arquivos já selecionados
    Tui(TuiArgs),
}

#[derive(Args, Debug)]
pub struct TuiArgs {
    /// Arquivos a selecionar ao abrir
    pub files: Vec<PathBuf>,
}

#[derive(Args, Debug)]
pub struct InfoArgs {
    pub input: PathBuf,
}

#[derive(Args, Debug)]
pub struct CutArgs {
    pub input: PathBuf,

    /// Início do trecho (SS[.ms], MM:SS[.ms] ou HH:MM:SS[.ms]); padrão: início do arquivo
    #[arg(long, allow_hyphen_values = true)]
    pub from: Option<String>,

    /// Fim do trecho; padrão: fim do arquivo
    #[arg(long, conflicts_with = "duration", allow_hyphen_values = true)]
    pub to: Option<String>,

    /// Duração do trecho (alternativa a --to)
    #[arg(long, allow_hyphen_values = true)]
    pub duration: Option<String>,

    /// Corte preciso ao frame (recodifica o vídeo do trecho)
    #[arg(long)]
    pub exact: bool,

    /// Arquivo de saída; padrão: <nome>.cut.<ext> ao lado da entrada
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ConvertArgs {
    pub input: PathBuf,

    /// Formato de destino (mp4, mkv, mov, webm, m4a, mp3, opus, flac); padrão: extensão de -o
    #[arg(long = "to")]
    pub format: Option<String>,

    /// Faixa de áudio a extrair em formatos somente de áudio (começa em 1)
    #[arg(long, value_name = "N")]
    pub audio_track: Option<usize>,

    /// Recodifica áudio Opus/FLAC para AAC em mp4/mov (máxima compatibilidade com players)
    #[arg(long)]
    pub compat: bool,

    /// Arquivo de saída; padrão: <nome>.<formato> ao lado da entrada
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct CompressArgs {
    pub input: PathBuf,

    /// Perfil de qualidade
    #[arg(long, value_enum, default_value_t = QualityArg::High)]
    pub quality: QualityArg,

    /// Codec de vídeo de saída
    #[arg(long, value_enum, default_value_t = CodecArg::Av1)]
    pub codec: CodecArg,

    /// Usa o encoder de hardware (NVENC > QSV > VAAPI): bem mais rápido, arquivo um pouco maior
    #[arg(long)]
    pub gpu: bool,

    /// Recodifica o áudio para Opus (ou AAC, se o container exigir) neste bitrate em kbps
    #[arg(long, value_name = "KBPS")]
    pub audio_bitrate: Option<u32>,

    /// Mede o VMAF da saída em relação à entrada ao final
    #[arg(long)]
    pub verify: bool,

    /// Mantém a saída mesmo que ela fique maior que a entrada
    #[arg(long)]
    pub force: bool,

    /// Arquivo de saída; padrão: <nome>.compressed.<ext> ao lado da entrada
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityArg {
    VisuallyLossless,
    High,
    Balanced,
    Small,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecArg {
    Av1,
    Hevc,
    H264,
}
