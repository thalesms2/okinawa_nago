//! Fila sequencial de jobs (spec `job-queue`).

use std::path::PathBuf;

use nago_core::output::Reservations;
use nago_core::report::Report;
use nago_core::runner::Progress;

use crate::opform::OpRequest;

pub type JobId = u64;

/// Por que um job terminou sem relatório.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailKind {
    /// Compressão que não reduziria o tamanho.
    Discarded,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobFailure {
    pub kind: FailKind,
    pub message: String,
    pub log_tail: Vec<String>,
}

impl JobFailure {
    pub fn from_error(err: &nago_core::Error) -> JobFailure {
        let (kind, log_tail) = match err {
            nago_core::Error::NotSmaller { .. } => (FailKind::Discarded, Vec::new()),
            nago_core::Error::Cancelled => (FailKind::Cancelled, Vec::new()),
            nago_core::Error::FfmpegFailed { log_tail, .. } => (FailKind::Failed, log_tail.clone()),
            _ => (FailKind::Failed, Vec::new()),
        };
        JobFailure {
            kind,
            message: err.to_string(),
            log_tail,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    Pending,
    /// Revalidando a operação imediatamente antes de executar.
    Preparing,
    Running {
        phase: String,
        progress: Option<Progress>,
    },
    Done(Report),
    Discarded(String),
    Failed {
        message: String,
        log_tail: Vec<String>,
    },
    Cancelled,
}

impl JobState {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            JobState::Pending | JobState::Preparing | JobState::Running { .. }
        )
    }

    pub fn is_running(&self) -> bool {
        matches!(self, JobState::Preparing | JobState::Running { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            JobState::Pending => "pendente",
            JobState::Preparing => "preparando",
            JobState::Running { .. } => "em execução",
            JobState::Done(_) => "concluído",
            JobState::Discarded(_) => "descartado",
            JobState::Failed { .. } => "falhou",
            JobState::Cancelled => "cancelado",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: JobId,
    pub label: String,
    pub request: OpRequest,
    /// Saída prometida no envio (reservada enquanto o job estiver ativo).
    pub planned_output: PathBuf,
    /// Saída real, conhecida ao começar a executar.
    pub output: Option<PathBuf>,
    pub state: JobState,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Queue {
    pub jobs: Vec<Job>,
    next_id: JobId,
    pub cursor: usize,
}

impl Queue {
    /// Acrescenta um job pendente no fim da fila.
    pub fn push(&mut self, request: OpRequest, planned_output: PathBuf) -> JobId {
        self.next_id += 1;
        let id = self.next_id;
        self.jobs.push(Job {
            id,
            label: request.label(),
            request,
            planned_output,
            output: None,
            state: JobState::Pending,
            warnings: Vec::new(),
        });
        id
    }

    pub fn get(&self, id: JobId) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn get_mut(&mut self, id: JobId) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    pub fn running(&self) -> Option<&Job> {
        self.jobs.iter().find(|j| j.state.is_running())
    }

    /// Próximo job a executar: o pendente mais antigo, se nenhum estiver rodando.
    pub fn next_to_start(&self) -> Option<JobId> {
        if self.running().is_some() {
            return None;
        }
        self.jobs
            .iter()
            .find(|j| j.state == JobState::Pending)
            .map(|j| j.id)
    }

    pub fn pending_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|j| j.state == JobState::Pending)
            .count()
    }

    pub fn active_count(&self) -> usize {
        self.jobs.iter().filter(|j| j.state.is_active()).count()
    }

    /// Saídas prometidas pelos jobs ativos, exceto `except`.
    pub fn reservations(&self, except: Option<JobId>) -> Reservations {
        let mut r = Reservations::new();
        for j in &self.jobs {
            if j.state.is_active() && Some(j.id) != except {
                r.insert(j.output.as_ref().unwrap_or(&j.planned_output));
            }
        }
        r
    }

    pub fn selected(&self) -> Option<&Job> {
        self.jobs.get(self.cursor)
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.jobs.len();
        self.cursor = if n == 0 {
            0
        } else {
            self.cursor.saturating_add_signed(delta).min(n - 1)
        };
    }

    /// Remove um job pendente ou terminado. Jobs em execução não são removidos.
    pub fn remove(&mut self, id: JobId) -> bool {
        let Some(i) = self.jobs.iter().position(|j| j.id == id) else {
            return false;
        };
        if self.jobs[i].state.is_running() {
            return false;
        }
        self.jobs.remove(i);
        self.move_cursor(0);
        true
    }

    /// Descarta todos os pendentes (saída confirmada); devolve quantos eram.
    pub fn drop_pending(&mut self) -> usize {
        let before = self.jobs.len();
        self.jobs.retain(|j| j.state != JobState::Pending);
        self.move_cursor(0);
        before - self.jobs.len()
    }
}
