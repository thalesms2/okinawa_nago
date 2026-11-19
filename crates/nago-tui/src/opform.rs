//! Formulários das operações (spec `interactive-mode` › Formulários) e estado do corte
//! (spec `frame-preview`).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use image::DynamicImage;
use nago_core::container::Container;
use nago_core::display;
use nago_core::encoders::Codec;
use nago_core::ops::{
    CompressRequest, ConvertRequest, CutRequest, OutputOptions, Prepared, PreparedKind,
};
use nago_core::preview::KeyframeIndex;
use nago_core::probe::MediaInfo;
use nago_core::profiles::Quality;
use nago_core::timefmt;
use ratatui_image::protocol::Protocol;

use crate::form::{Field, FieldId, Form, TextKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Cut,
    Convert,
    Compress,
}

impl OpKind {
    pub fn verb(self) -> &'static str {
        match self {
            OpKind::Cut => "cortar",
            OpKind::Convert => "converter",
            OpKind::Compress => "comprimir",
        }
    }
}

/// Uma operação configurada para um arquivo.
#[derive(Debug, Clone)]
pub enum OpRequest {
    Compress(CompressRequest),
    Convert(ConvertRequest),
    Cut(CutRequest),
}

impl OpRequest {
    pub fn kind(&self) -> OpKind {
        match self {
            OpRequest::Compress(_) => OpKind::Compress,
            OpRequest::Convert(_) => OpKind::Convert,
            OpRequest::Cut(_) => OpKind::Cut,
        }
    }

    pub fn input(&self) -> &Path {
        match self {
            OpRequest::Compress(r) => &r.input,
            OpRequest::Convert(r) => &r.input,
            OpRequest::Cut(r) => &r.input,
        }
    }

    /// Rótulo curto para a fila (`comprimir video.mp4`).
    pub fn label(&self) -> String {
        let name = self
            .input()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{} {name}", self.kind().verb())
    }

    /// Fixa a saída num caminho já resolvido (sem sobrescrita implícita).
    pub fn with_output(mut self, path: PathBuf) -> OpRequest {
        let out = OutputOptions {
            path: Some(path),
            overwrite: self.output().overwrite,
        };
        match &mut self {
            OpRequest::Compress(r) => r.output = out,
            OpRequest::Convert(r) => r.output = out,
            OpRequest::Cut(r) => r.output = out,
        }
        self
    }

    pub fn output(&self) -> &OutputOptions {
        match self {
            OpRequest::Compress(r) => &r.output,
            OpRequest::Convert(r) => &r.output,
            OpRequest::Cut(r) => &r.output,
        }
    }
}

/// O que mostrar de uma operação preparada.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub output: PathBuf,
    pub encoder: Option<String>,
    pub quality: Option<Quality>,
    pub warnings: Vec<String>,
    pub streams: Vec<String>,
}

impl From<&Prepared> for Summary {
    fn from(p: &Prepared) -> Summary {
        let report = nago_core::report::Report::new(p.operation(), &p.plan);
        let streams = report
            .streams
            .iter()
            .map(display::describe_stream)
            .collect();
        let quality = match p.kind {
            PreparedKind::Compress { quality, .. } => Some(quality),
            _ => p.plan.quality,
        };
        Summary {
            output: p.output.clone(),
            encoder: p.encoder().map(str::to_owned),
            quality,
            warnings: p.warnings().to_vec(),
            streams,
        }
    }
}

/// Resultado da validação do formulário pelo core.
#[derive(Debug, Clone, PartialEq)]
pub enum Check {
    Checking,
    Ready(Summary),
    Invalid(String),
}

// ---------------------------------------------------------------------------
// Corte

#[derive(Debug, Clone, PartialEq)]
pub enum Keyframes {
    Indexing,
    Ready(KeyframeIndex),
    /// Arquivo sem vídeo.
    NoVideo,
    Failed(String),
}

/// Chave do cache de miniaturas: instante em ms e caixa em pixels.
pub type FrameKey = (u64, u32, u32);

/// Cache LRU pequeno de frames decodificados.
#[derive(Default)]
pub struct FrameCache {
    items: VecDeque<(FrameKey, DynamicImage)>,
}

impl FrameCache {
    pub const CAPACITY: usize = 32;

    pub fn get(&mut self, key: FrameKey) -> Option<DynamicImage> {
        let i = self.items.iter().position(|(k, _)| *k == key)?;
        let item = self.items.remove(i)?;
        let image = item.1.clone();
        self.items.push_back(item);
        Some(image)
    }

    pub fn insert(&mut self, key: FrameKey, image: DynamicImage) {
        self.items.retain(|(k, _)| *k != key);
        if self.items.len() == Self::CAPACITY {
            self.items.pop_front();
        }
        self.items.push_back((key, image));
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Frame exibido na miniatura.
pub struct Shown {
    pub key: FrameKey,
    /// Protocolo pronto para desenhar (ausente quando não há `Picker`, como nos testes).
    pub protocol: Option<Protocol>,
}

#[derive(Default)]
pub struct Thumb {
    /// Sequência do último pedido: respostas de sequências antigas são descartadas.
    pub seq: u64,
    pub shown: Option<Shown>,
    pub loading: bool,
    pub error: Option<String>,
    pub cache: FrameCache,
}

pub struct CutState {
    pub duration: f64,
    pub fps: Option<f64>,
    pub keyframes: Keyframes,
    pub thumb: Thumb,
}

impl CutState {
    pub fn new(info: &MediaInfo) -> CutState {
        let video = info.primary_video();
        CutState {
            duration: info.duration_secs,
            fps: video.and_then(|v| v.video.as_ref()).and_then(|v| v.fps),
            keyframes: if video.is_some() {
                Keyframes::Indexing
            } else {
                Keyframes::NoVideo
            },
            thumb: Thumb::default(),
        }
    }

    pub fn has_video(&self) -> bool {
        self.keyframes != Keyframes::NoVideo
    }

    pub fn frame_step(&self) -> f64 {
        1.0 / self.fps.filter(|f| *f > 0.0).unwrap_or(30.0)
    }

    pub fn index(&self) -> Option<&KeyframeIndex> {
        match &self.keyframes {
            Keyframes::Ready(i) => Some(i),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------

pub struct OpForm {
    pub kind: OpKind,
    pub files: Vec<PathBuf>,
    pub form: Form,
    /// Sequência da última validação pedida ao core.
    pub seq: u64,
    pub check: Check,
    pub last_edited: Option<FieldId>,
    /// Só no corte; `None` até o probe do primeiro arquivo chegar.
    pub cut: Option<CutState>,
    pub submitting: bool,
}

fn quality_names() -> Vec<&'static str> {
    Quality::ALL.map(Quality::name).to_vec()
}

impl OpForm {
    pub fn new(kind: OpKind, files: Vec<PathBuf>) -> OpForm {
        let output_hint = match kind {
            OpKind::Compress => "automático (<nome>.compressed.<ext>)",
            OpKind::Convert => "automático (<nome>.<formato>)",
            OpKind::Cut => "automático (<nome>.cut.<ext>)",
        };
        let mut fields = match kind {
            OpKind::Compress => vec![
                Field::choice(FieldId::Quality, "perfil", quality_names(), 1),
                Field::choice(FieldId::Codec, "codec", vec!["av1", "hevc", "h264"], 0),
                Field::toggle(FieldId::Gpu, "usar GPU", false),
                Field::toggle(FieldId::Verify, "medir VMAF", false),
                Field::text(
                    FieldId::AudioBitrate,
                    "áudio (kbps)",
                    TextKind::Digits,
                    "copiar sem recodificar",
                ),
                Field::toggle(FieldId::Force, "manter se maior", false),
            ],
            OpKind::Convert => vec![
                Field::choice(
                    FieldId::Format,
                    "formato",
                    Container::ALL.map(Container::ext).to_vec(),
                    0,
                ),
                Field::text(
                    FieldId::AudioTrack,
                    "faixa de áudio",
                    TextKind::Digits,
                    "todas",
                ),
                Field::toggle(FieldId::Compat, "compatibilidade", false),
            ],
            OpKind::Cut => vec![
                Field::text(FieldId::Start, "início", TextKind::Time, "00:00:00.000"),
                Field::text(FieldId::End, "fim", TextKind::Time, "fim do arquivo"),
                Field::toggle(FieldId::Exact, "exato ao frame", false),
            ],
        };
        fields.push(Field::text(
            FieldId::Output,
            "saída",
            TextKind::Any,
            output_hint,
        ));
        fields.push(Field::toggle(FieldId::Overwrite, "sobrescrever", false));
        OpForm {
            kind,
            files,
            form: Form::new(fields),
            seq: 0,
            check: Check::Checking,
            last_edited: None,
            cut: None,
            submitting: false,
        }
    }

    pub fn first_file(&self) -> &Path {
        &self.files[0]
    }

    /// Prepara o estado do corte com o probe do primeiro arquivo.
    pub fn init_cut(&mut self, info: &MediaInfo) {
        if self.kind != OpKind::Cut || self.cut.is_some() {
            return;
        }
        if self.form.text(FieldId::Start).is_empty() {
            self.form.set_text(FieldId::Start, timefmt::format(0.0));
        }
        if self.form.text(FieldId::End).is_empty() {
            self.form
                .set_text(FieldId::End, timefmt::format(info.duration_secs));
        }
        self.cut = Some(CutState::new(info));
    }

    fn time(&self, id: FieldId) -> Result<Option<f64>, String> {
        let text = self.form.text(id);
        if text.is_empty() {
            return Ok(None);
        }
        timefmt::parse(text).map(Some).map_err(|e| e.to_string())
    }

    /// Instante atual de um extremo do corte (vazio = início/fim do arquivo).
    pub fn edge(&self, id: FieldId) -> Option<f64> {
        match self.time(id) {
            Ok(Some(t)) => Some(t),
            Ok(None) => match id {
                FieldId::End => self.cut.as_ref().map(|c| c.duration),
                _ => Some(0.0),
            },
            Err(_) => None,
        }
    }

    /// Extremo do corte em edição: o campo em foco, ou o início.
    pub fn focused_edge(&self) -> FieldId {
        match self.form.focused().id {
            FieldId::End => FieldId::End,
            _ => FieldId::Start,
        }
    }

    pub fn set_edge(&mut self, id: FieldId, t: f64) {
        let max = self.cut.as_ref().map_or(f64::MAX, |c| c.duration);
        self.form.set_text(id, timefmt::format(t.clamp(0.0, max)));
    }

    /// Monta o pedido para `input` validando localmente os campos.
    /// Em caso de erro, devolve o campo e a mensagem.
    pub fn request(&self, input: &Path) -> Result<OpRequest, (FieldId, String)> {
        let f = &self.form;
        let out_text = f.text(FieldId::Output);
        let output = OutputOptions {
            path: (!out_text.is_empty()).then(|| PathBuf::from(out_text)),
            overwrite: f.toggle(FieldId::Overwrite),
        };
        let number = |id: FieldId| -> Result<Option<u32>, (FieldId, String)> {
            let t = f.text(id);
            if t.is_empty() {
                return Ok(None);
            }
            t.parse::<u32>()
                .map(Some)
                .map_err(|_| (id, format!("número inválido: {t}")))
        };
        Ok(match self.kind {
            OpKind::Compress => {
                let quality = Quality::ALL
                    .into_iter()
                    .find(|q| q.name() == f.choice(FieldId::Quality))
                    .unwrap_or(Quality::High);
                let codec = Codec::ALL
                    .into_iter()
                    .find(|c| c.ffprobe_name() == f.choice(FieldId::Codec))
                    .unwrap_or(Codec::Av1);
                OpRequest::Compress(CompressRequest {
                    input: input.to_path_buf(),
                    output,
                    quality,
                    codec,
                    gpu: f.toggle(FieldId::Gpu),
                    audio_bitrate: number(FieldId::AudioBitrate)?,
                    verify: f.toggle(FieldId::Verify),
                    force: f.toggle(FieldId::Force),
                })
            }
            OpKind::Convert => OpRequest::Convert(ConvertRequest {
                input: input.to_path_buf(),
                output,
                format: Some(f.choice(FieldId::Format).to_owned()),
                audio_track: number(FieldId::AudioTrack)?.map(|n| n as usize),
                compat: f.toggle(FieldId::Compat),
            }),
            OpKind::Cut => {
                let from = self.time(FieldId::Start).map_err(|e| (FieldId::Start, e))?;
                let to = self.time(FieldId::End).map_err(|e| (FieldId::End, e))?;
                OpRequest::Cut(CutRequest {
                    input: input.to_path_buf(),
                    output,
                    from,
                    to,
                    duration: None,
                    exact: f.toggle(FieldId::Exact),
                })
            }
        })
    }

    /// Indicação do corte rápido: (keyframe onde o trecho começa, diferença em s).
    pub fn fast_cut_start(&self) -> Option<(f64, f64)> {
        let cut = self.cut.as_ref()?;
        if self.form.toggle(FieldId::Exact) {
            return None;
        }
        let start = self.edge(FieldId::Start)?;
        let kf = cut.index()?.at_or_before(start)?;
        Some((kf, start - kf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_de_frames_descarta_o_mais_antigo() {
        let mut c = FrameCache::default();
        for i in 0..40u64 {
            c.insert((i, 1, 1), DynamicImage::new_rgb8(1, 1));
        }
        assert_eq!(c.len(), FrameCache::CAPACITY);
        assert!(c.get((0, 1, 1)).is_none());
        assert!(c.get((39, 1, 1)).is_some());
    }

    #[test]
    fn pedido_de_compressao_com_padroes() {
        let f = OpForm::new(OpKind::Compress, vec!["/v/a.mp4".into()]);
        let OpRequest::Compress(r) = f.request(Path::new("/v/a.mp4")).unwrap() else {
            panic!()
        };
        assert_eq!(r.quality, Quality::High);
        assert_eq!(r.codec, Codec::Av1);
        assert!(!r.gpu && !r.verify && !r.force);
        assert_eq!(r.output.path, None);
    }

    #[test]
    fn tempo_invalido_aponta_o_campo() {
        let mut f = OpForm::new(OpKind::Cut, vec!["/v/a.mp4".into()]);
        f.form.set_text(FieldId::Start, "1:2:3:4".into());
        let (field, msg) = f.request(Path::new("/v/a.mp4")).unwrap_err();
        assert_eq!(field, FieldId::Start);
        assert!(!msg.is_empty());
    }
}
