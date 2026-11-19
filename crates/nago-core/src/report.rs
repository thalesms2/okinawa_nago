//! Relatório final das operações (spec `job-execution` › Relatório final).

use std::path::PathBuf;

use serde::Serialize;

use crate::plan::{Action, Plan};
use crate::probe::StreamKind;
use crate::profiles::Quality;

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub operation: &'static str,
    pub input: PathBuf,
    pub output: PathBuf,
    pub input_size_bytes: u64,
    pub output_size_bytes: u64,
    /// Variação de tamanho em % (negativo = menor).
    pub size_change_pct: f64,
    pub elapsed_secs: f64,
    /// Encoder de vídeo usado (`null` quando nada foi recodificado).
    pub encoder: Option<String>,
    pub quality: Option<Quality>,
    pub streams: Vec<StreamReport>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_start_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_start_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vmaf_mean: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vmaf_p5: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vmaf_target: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamReport {
    pub index: usize,
    pub kind: StreamKind,
    pub codec_in: String,
    /// `copy`, `transcode` ou `drop`.
    pub action: &'static str,
    pub codec_out: Option<String>,
    pub reason: Option<String>,
}

impl Report {
    pub fn new(operation: &'static str, plan: &Plan) -> Report {
        Report {
            operation,
            input: plan.input.clone(),
            output: PathBuf::new(),
            input_size_bytes: 0,
            output_size_bytes: 0,
            size_change_pct: 0.0,
            elapsed_secs: 0.0,
            encoder: plan.video_encoder.clone(),
            quality: plan.quality,
            streams: plan.streams.iter().map(StreamReport::from_plan).collect(),
            warnings: plan.warnings.clone(),
            requested_start_secs: None,
            actual_start_secs: None,
            duration_secs: None,
            vmaf_mean: None,
            vmaf_p5: None,
            vmaf_target: None,
        }
    }

    pub fn set_sizes(&mut self, input: u64, output: u64) {
        self.input_size_bytes = input;
        self.output_size_bytes = output;
        self.size_change_pct = if input > 0 {
            (output as f64 - input as f64) / input as f64 * 100.0
        } else {
            0.0
        };
    }
}

impl StreamReport {
    fn from_plan(s: &crate::plan::StreamPlan) -> StreamReport {
        let (action, codec_out, reason) = match &s.action {
            Action::Copy => ("copy", Some(s.codec_in.clone()), None),
            Action::Transcode(e) => ("transcode", Some(e.encoder.clone()), s.note.clone()),
            Action::Drop { reason } => ("drop", None, Some(reason.clone())),
        };
        StreamReport {
            index: s.index,
            kind: s.kind,
            codec_in: s.codec_in.clone(),
            action,
            codec_out,
            reason,
        }
    }
}
