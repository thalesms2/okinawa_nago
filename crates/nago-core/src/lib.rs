//! Núcleo do `nago`: inspeção de mídia, planejamento e execução de jobs do ffmpeg.
//!
//! Esta crate não faz I/O de terminal: ela recebe opções, devolve planos e
//! emite [`runner::JobEvent`]s, que a CLI (ou a futura TUI) renderiza.

pub mod caps;
pub mod container;
pub mod encoders;
pub mod error;
pub mod ops;
pub mod output;
pub mod plan;
pub mod probe;
pub mod profiles;
pub mod report;
pub mod runner;
pub mod timefmt;
pub mod tools;
pub mod vmaf;

pub use error::{Error, Result};
