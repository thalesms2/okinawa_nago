//! Estado da TUI e transições (`update`), sem I/O.
//!
//! `update` recebe uma [`Msg`] (tecla, resultado de um efeito, evento de job) e devolve
//! os [`Effect`]s a executar. O tempo entra pelas mensagens (`Key`, `Tick`), então os
//! atrasos (debounce) também são testáveis sem relógio.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use image::DynamicImage;
use nago_core::caps::Capabilities;
use nago_core::output::Reservations;
use nago_core::preview::KeyframeIndex;
use nago_core::probe::MediaInfo;
use nago_core::report::Report;
use nago_core::runner::JobEvent;
use ratatui::layout::Rect;
use ratatui_image::Resize;
use ratatui_image::picker::Picker;

use crate::browser::{Browser, Entry};
use crate::form::{FieldId, FieldKind, FormEvent, TextKind};
use crate::keys::HelpScreen;
use crate::layout;
use crate::opform::{Check, FrameKey, Keyframes, OpForm, OpKind, OpRequest, Shown, Summary};
use crate::queue::{FailKind, JobFailure, JobId, JobState, Queue};

pub const PROBE_DELAY: Duration = Duration::from_millis(150);
pub const PREPARE_DELAY: Duration = Duration::from_millis(300);
pub const FRAME_DELAY: Duration = Duration::from_millis(120);
pub const CTRL_C_WINDOW: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum Msg {
    Key(KeyEvent, Instant),
    Tick(Instant),
    Resize(u16, u16),
    /// SIGTERM.
    Terminate,
    DirListed {
        dir: PathBuf,
        result: Result<Vec<Entry>, String>,
    },
    Probed {
        path: PathBuf,
        result: Result<MediaInfo, String>,
    },
    Prepared {
        seq: u64,
        result: Result<Summary, String>,
    },
    Enqueued {
        results: Vec<(OpRequest, Result<PathBuf, String>)>,
    },
    KeyframesIndexed {
        path: PathBuf,
        result: Result<Option<KeyframeIndex>, String>,
    },
    Frame {
        seq: u64,
        key: FrameKey,
        result: Result<DynamicImage, String>,
    },
    JobStarted {
        id: JobId,
        output: PathBuf,
    },
    Job {
        id: JobId,
        event: JobEvent,
    },
    JobFinished {
        id: JobId,
        result: Result<Report, JobFailure>,
    },
}

#[derive(Debug)]
pub enum Effect {
    ListDir(PathBuf),
    Probe(PathBuf),
    /// Valida a operação no core (sem executar).
    Prepare {
        seq: u64,
        request: OpRequest,
        reserved: Reservations,
    },
    /// Prepara cada pedido acumulando as reservas e devolve as saídas resolvidas.
    Enqueue {
        requests: Vec<OpRequest>,
        reserved: Reservations,
    },
    IndexKeyframes(PathBuf),
    Frame {
        seq: u64,
        path: PathBuf,
        t: f64,
        key: FrameKey,
    },
    StartJob {
        id: JobId,
        request: OpRequest,
        reserved: Reservations,
    },
    CancelJob(JobId),
    Exit(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Browser,
    Form,
    Queue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    Help,
    ConfirmQuit,
}

#[derive(Debug, Clone)]
pub enum InfoState {
    Loading,
    Ready(MediaInfo),
    Failed(String),
}

/// Suporte a miniaturas: presente só em terminais com protocolo de imagem.
pub struct ThumbSupport {
    /// Ausente nos testes: o frame é registrado, mas não há protocolo para desenhar.
    pub picker: Option<Picker>,
    /// Tamanho da célula em pixels.
    pub font: (u16, u16),
}

pub struct AppInit {
    pub dir: PathBuf,
    pub files: Vec<PathBuf>,
    pub caps: Capabilities,
    pub thumbs: Option<ThumbSupport>,
    pub size: (u16, u16),
}

pub struct App {
    pub screen: Screen,
    pub overlay: Option<Overlay>,
    pub browser: Browser,
    pub form: Option<OpForm>,
    pub queue: Queue,
    pub infos: HashMap<PathBuf, InfoState>,
    pub caps: Capabilities,
    pub thumbs: Option<ThumbSupport>,
    pub size: (u16, u16),
    /// Mensagem temporária no rodapé (limpa na próxima tecla).
    pub status: Option<String>,
    /// Código de saída a usar quando o job cancelado terminar.
    pub quitting: Option<u8>,
    last_ctrl_c: Option<Instant>,
    pending_probe: Option<(Instant, PathBuf)>,
    pending_prepare: Option<Instant>,
    pending_frame: Option<Instant>,
}

impl App {
    pub fn new(init: AppInit) -> (App, Vec<Effect>) {
        let mut browser = Browser::new(init.dir);
        if let Some(first) = init.files.first() {
            if let Some(parent) = first.parent() {
                browser.dir = parent.to_path_buf();
            }
            browser.select_on_load = Some(first.clone());
            browser.marked = init.files.iter().cloned().collect();
        }
        let effects = vec![Effect::ListDir(browser.dir.clone())];
        let app = App {
            screen: Screen::Browser,
            overlay: None,
            browser,
            form: None,
            queue: Queue::default(),
            infos: HashMap::new(),
            caps: init.caps,
            thumbs: init.thumbs,
            size: init.size,
            status: None,
            quitting: None,
            last_ctrl_c: None,
            pending_probe: None,
            pending_prepare: None,
            pending_frame: None,
        };
        (app, effects)
    }

    /// Próximo instante em que um efeito adiado deve ser disparado.
    pub fn next_deadline(&self) -> Option<Instant> {
        [
            self.pending_probe.as_ref().map(|(t, _)| *t),
            self.pending_prepare,
            self.pending_frame,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Tela atual para a ajuda.
    pub fn help_screen(&self) -> HelpScreen {
        match (self.screen, &self.form) {
            (Screen::Form, Some(f)) if f.kind == OpKind::Cut => HelpScreen::Cut,
            (Screen::Form, _) => HelpScreen::Form,
            (Screen::Queue, _) => HelpScreen::Queue,
            (Screen::Browser, _) => HelpScreen::Browser,
        }
    }

    pub fn info(&self, path: &std::path::Path) -> Option<&InfoState> {
        self.infos.get(path)
    }

    /// O campo em foco aceita texto livre (então `?`, `q` etc. são digitados).
    fn typing_free_text(&self) -> bool {
        self.screen == Screen::Form
            && self.form.as_ref().is_some_and(|f| {
                matches!(
                    f.form.focused().kind,
                    FieldKind::Text {
                        kind: TextKind::Any,
                        ..
                    }
                )
            })
    }
}

pub fn update(app: &mut App, msg: Msg) -> Vec<Effect> {
    match msg {
        Msg::Key(key, now) => on_key(app, key, now),
        Msg::Tick(now) => fire_due(app, now),
        Msg::Resize(w, h) => {
            app.size = (w, h);
            Vec::new()
        }
        Msg::Terminate => confirm_quit(app),
        Msg::DirListed { dir, result } => on_dir_listed(app, dir, result),
        Msg::Probed { path, result } => on_probed(app, path, result),
        Msg::Prepared { seq, result } => {
            on_prepared(app, seq, result);
            Vec::new()
        }
        Msg::Enqueued { results } => on_enqueued(app, results),
        Msg::KeyframesIndexed { path, result } => {
            on_keyframes(app, path, result);
            Vec::new()
        }
        Msg::Frame { seq, key, result } => {
            on_frame(app, seq, key, result);
            Vec::new()
        }
        Msg::JobStarted { id, output } => {
            if let Some(job) = app.queue.get_mut(id) {
                job.output = Some(output);
                job.state = JobState::Running {
                    phase: String::new(),
                    progress: None,
                };
            }
            Vec::new()
        }
        Msg::Job { id, event } => {
            on_job_event(app, id, event);
            Vec::new()
        }
        Msg::JobFinished { id, result } => on_job_finished(app, id, result),
    }
}

// ---------------------------------------------------------------------------
// Teclado

fn on_key(app: &mut App, key: KeyEvent, now: Instant) -> Vec<Effect> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return on_ctrl_c(app, now);
    }
    app.status = None;
    match app.overlay {
        Some(Overlay::Help) => {
            if matches!(key.code, KeyCode::Char('?' | 'q') | KeyCode::Esc) {
                app.overlay = None;
            }
            return Vec::new();
        }
        Some(Overlay::ConfirmQuit) => {
            return match key.code {
                KeyCode::Char('s' | 'S' | 'y' | 'Y') | KeyCode::Enter => confirm_quit(app),
                KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                    app.overlay = None;
                    Vec::new()
                }
                _ => Vec::new(),
            };
        }
        None => {}
    }
    if key.code == KeyCode::Char('?') && !app.typing_free_text() {
        app.overlay = Some(Overlay::Help);
        return Vec::new();
    }
    match app.screen {
        Screen::Browser => browser_key(app, key, now),
        Screen::Form => form_key(app, key, now),
        Screen::Queue => queue_key(app, key, now),
    }
}

fn on_ctrl_c(app: &mut App, now: Instant) -> Vec<Effect> {
    let repeated = app
        .last_ctrl_c
        .is_some_and(|t| now.saturating_duration_since(t) <= CTRL_C_WINDOW);
    app.last_ctrl_c = Some(now);
    if repeated && app.overlay == Some(Overlay::ConfirmQuit) {
        return confirm_quit(app);
    }
    request_quit(app)
}

fn request_quit(app: &mut App) -> Vec<Effect> {
    if app.queue.active_count() == 0 {
        return vec![Effect::Exit(0)];
    }
    app.overlay = Some(Overlay::ConfirmQuit);
    Vec::new()
}

/// Saída confirmada: descarta pendentes e cancela o job em execução.
fn confirm_quit(app: &mut App) -> Vec<Effect> {
    app.overlay = None;
    let dropped = app.queue.drop_pending();
    let running = app.queue.running().map(|j| j.id);
    let code = if dropped > 0 || running.is_some() {
        130
    } else {
        0
    };
    match running {
        Some(id) => {
            app.quitting = Some(code);
            app.status = Some("cancelando o job em execução antes de sair…".into());
            vec![Effect::CancelJob(id)]
        }
        None => vec![Effect::Exit(code)],
    }
}

fn browser_key(app: &mut App, key: KeyEvent, now: Instant) -> Vec<Effect> {
    let b = &mut app.browser;
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            b.move_cursor(-1);
            schedule_probe(app, now);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            b.move_cursor(1);
            schedule_probe(app, now);
        }
        KeyCode::PageUp => {
            b.move_cursor(-10);
            schedule_probe(app, now);
        }
        KeyCode::PageDown => {
            b.move_cursor(10);
            schedule_probe(app, now);
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
            if let Some(e) = b.selected().filter(|e| e.is_dir) {
                if e.name == ".." {
                    return go_parent(app);
                }
                return enter_dir(app, e.path, None);
            }
        }
        KeyCode::Backspace | KeyCode::Char('h') | KeyCode::Left => return go_parent(app),
        KeyCode::Char(' ') => {
            b.toggle_mark();
            b.move_cursor(1);
            schedule_probe(app, now);
        }
        KeyCode::Char('.') => b.toggle_show_all(),
        KeyCode::Char('c') => return open_form(app, OpKind::Cut, now),
        KeyCode::Char('v') => return open_form(app, OpKind::Convert, now),
        KeyCode::Char('z') => return open_form(app, OpKind::Compress, now),
        KeyCode::Tab => app.screen = Screen::Queue,
        KeyCode::Char('q') => return request_quit(app),
        _ => {}
    }
    Vec::new()
}

fn enter_dir(app: &mut App, dir: PathBuf, select: Option<PathBuf>) -> Vec<Effect> {
    let b = &mut app.browser;
    b.dir = dir.clone();
    b.entries.clear();
    b.cursor = 0;
    b.loading = true;
    b.select_on_load = select;
    app.pending_probe = None;
    vec![Effect::ListDir(dir)]
}

fn go_parent(app: &mut App) -> Vec<Effect> {
    let current = app.browser.dir.clone();
    match current.parent() {
        Some(parent) => enter_dir(app, parent.to_path_buf(), Some(current)),
        None => Vec::new(),
    }
}

/// Agenda o probe do arquivo destacado (adiado para não disparar um por linha ao rolar).
fn schedule_probe(app: &mut App, now: Instant) {
    let Some(path) = app.browser.selected_file() else {
        app.pending_probe = None;
        return;
    };
    match app.infos.get(&path) {
        Some(InfoState::Ready(_) | InfoState::Failed(_)) => app.pending_probe = None,
        _ => {
            app.infos.insert(path.clone(), InfoState::Loading);
            app.pending_probe = Some((now + PROBE_DELAY, path));
        }
    }
}

fn on_dir_listed(app: &mut App, dir: PathBuf, result: Result<Vec<Entry>, String>) -> Vec<Effect> {
    if dir != app.browser.dir {
        return Vec::new();
    }
    match result {
        Ok(entries) => app.browser.set_entries(entries),
        Err(e) => {
            app.browser.loading = false;
            app.browser.error = Some(e);
        }
    }
    match app.browser.selected_file() {
        Some(path) if !app.infos.contains_key(&path) => {
            app.infos.insert(path.clone(), InfoState::Loading);
            vec![Effect::Probe(path)]
        }
        _ => Vec::new(),
    }
}

fn on_probed(app: &mut App, path: PathBuf, result: Result<MediaInfo, String>) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let (Ok(info), Some(form)) = (&result, app.form.as_mut())
        && form.kind == OpKind::Cut
        && form.cut.is_none()
        && form.first_file() == path
    {
        form.init_cut(info);
        if form.cut.as_ref().is_some_and(|c| c.has_video()) {
            effects.push(Effect::IndexKeyframes(path.clone()));
        }
        effects.extend(fire_prepare(app));
        effects.extend(fire_frame(app));
    }
    let state = match result {
        Ok(info) => InfoState::Ready(info),
        Err(e) => InfoState::Failed(e),
    };
    app.infos.insert(path, state);
    effects
}

// ---------------------------------------------------------------------------
// Formulários

fn open_form(app: &mut App, kind: OpKind, now: Instant) -> Vec<Effect> {
    let files = app.browser.targets();
    if files.is_empty() {
        app.status = Some("selecione um arquivo (ou marque vários com Espaço)".into());
        return Vec::new();
    }
    let first = files[0].clone();
    let mut form = OpForm::new(kind, files);
    let mut effects = Vec::new();
    match app.infos.get(&first) {
        Some(InfoState::Ready(info)) if kind == OpKind::Cut => {
            form.init_cut(info);
            if form.cut.as_ref().is_some_and(|c| c.has_video()) {
                effects.push(Effect::IndexKeyframes(first.clone()));
            }
            app.pending_frame = Some(now);
        }
        Some(InfoState::Ready(_) | InfoState::Failed(_)) => {}
        // um probe adiado do navegador vira imediato: o painel e o corte dependem dele
        _ => {
            app.pending_probe = None;
            app.infos.insert(first.clone(), InfoState::Loading);
            effects.push(Effect::Probe(first.clone()));
        }
    }
    app.form = Some(form);
    app.screen = Screen::Form;
    app.pending_prepare = Some(now);
    effects
}

fn close_form(app: &mut App) {
    app.form = None;
    app.screen = Screen::Browser;
    app.pending_prepare = None;
    app.pending_frame = None;
}

fn form_key(app: &mut App, key: KeyEvent, now: Instant) -> Vec<Effect> {
    let Some(form) = app.form.as_mut() else {
        app.screen = Screen::Browser;
        return Vec::new();
    };
    match key.code {
        KeyCode::Esc => {
            close_form(app);
            return Vec::new();
        }
        KeyCode::Enter => return submit(app),
        _ => {}
    }
    if form.kind == OpKind::Cut
        && let Some(effects) = cut_key(app, key, now)
    {
        return effects;
    }
    let Some(form) = app.form.as_mut() else {
        return Vec::new();
    };
    let edge_before = form.focused_edge();
    match form.form.handle_key(key) {
        FormEvent::Ignored => {}
        FormEvent::Moved => {
            if form.kind == OpKind::Cut && form.focused_edge() != edge_before {
                schedule_frame(app, now);
            }
        }
        FormEvent::Changed(id) => on_changed(app, id, now),
    }
    Vec::new()
}

/// Teclas próprias do corte. `None` quando a tecla deve ir para o formulário.
fn cut_key(app: &mut App, key: KeyEvent, now: Instant) -> Option<Vec<Effect>> {
    let form = app.form.as_mut()?;
    let focus = form.form.focused().id;
    let on_edge = matches!(focus, FieldId::Start | FieldId::End);
    let free_text = matches!(
        form.form.focused().kind,
        FieldKind::Text {
            kind: TextKind::Any,
            ..
        }
    );
    if free_text {
        return None;
    }
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let edge = form.focused_edge();
    enum Step {
        By(f64),
        Frames(f64),
        PrevKey,
        NextKey,
    }
    let step = match key.code {
        KeyCode::Char('i') => {
            form.form.focus_on(FieldId::Start);
            schedule_frame(app, now);
            return Some(Vec::new());
        }
        KeyCode::Char('o') => {
            form.form.focus_on(FieldId::End);
            schedule_frame(app, now);
            return Some(Vec::new());
        }
        KeyCode::Left if on_edge => Step::By(if shift { -10.0 } else { -1.0 }),
        KeyCode::Right if on_edge => Step::By(if shift { 10.0 } else { 1.0 }),
        KeyCode::Char('<') => Step::Frames(-1.0),
        KeyCode::Char('>') => Step::Frames(1.0),
        KeyCode::Char('[') => Step::PrevKey,
        KeyCode::Char(']') => Step::NextKey,
        _ => return None,
    };
    let cut = form.cut.as_ref()?;
    let current = form.edge(edge)?;
    let target = match step {
        Step::By(d) => Some(current + d),
        // alinha à grade de frames: o texto tem precisão de ms e somar passos acumularia erro
        Step::Frames(n) => {
            let step = cut.frame_step();
            Some(((current / step).round() + n) * step)
        }
        Step::PrevKey => cut.index().and_then(|i| i.before(current)),
        Step::NextKey => cut.index().and_then(|i| i.after(current)),
    };
    let Some(target) = target else {
        if cut.index().is_none() && cut.has_video() {
            app.status = Some("aguarde: indexando keyframes…".into());
        }
        return Some(Vec::new());
    };
    form.form.focus_on(edge);
    form.set_edge(edge, target);
    on_changed(app, edge, now);
    Some(Vec::new())
}

/// Revalida após uma edição: erros locais na hora, validação do core adiada.
fn on_changed(app: &mut App, id: FieldId, now: Instant) {
    let Some(form) = app.form.as_mut() else {
        return;
    };
    form.last_edited = Some(id);
    form.form.clear_errors();
    match form.request(&form.files[0].clone()) {
        Err((field, msg)) => {
            form.form.set_error(field, Some(msg.clone()));
            form.check = Check::Invalid(msg);
            app.pending_prepare = None;
        }
        Ok(_) => {
            form.check = Check::Checking;
            app.pending_prepare = Some(now + PREPARE_DELAY);
        }
    }
    if form.kind == OpKind::Cut && matches!(id, FieldId::Start | FieldId::End) {
        schedule_frame(app, now);
    }
}

fn fire_prepare(app: &mut App) -> Vec<Effect> {
    app.pending_prepare = None;
    let reserved = app.queue.reservations(None);
    let Some(form) = app.form.as_mut() else {
        return Vec::new();
    };
    match form.request(&form.files[0].clone()) {
        Ok(request) => {
            form.seq += 1;
            form.check = Check::Checking;
            vec![Effect::Prepare {
                seq: form.seq,
                request,
                reserved,
            }]
        }
        Err((field, msg)) => {
            form.form.set_error(field, Some(msg.clone()));
            form.check = Check::Invalid(msg);
            Vec::new()
        }
    }
}

/// Campo onde mostrar um erro do core.
fn field_for_error(form: &OpForm, msg: &str) -> FieldId {
    if msg.contains("outro job da fila") || msg.contains("já existe") || msg.contains("-o") {
        return FieldId::Output;
    }
    form.last_edited.unwrap_or_else(|| form.form.fields[0].id)
}

fn on_prepared(app: &mut App, seq: u64, result: Result<Summary, String>) {
    let Some(form) = app.form.as_mut() else {
        return;
    };
    if form.seq != seq {
        return;
    }
    form.form.clear_errors();
    form.check = match result {
        Ok(summary) => Check::Ready(summary),
        Err(msg) => {
            let field = field_for_error(form, &msg);
            form.form.set_error(field, Some(msg.clone()));
            Check::Invalid(msg)
        }
    };
}

fn submit(app: &mut App) -> Vec<Effect> {
    let reserved = app.queue.reservations(None);
    let Some(form) = app.form.as_mut() else {
        return Vec::new();
    };
    if form.submitting {
        return Vec::new();
    }
    if form.form.first_error().is_some() || matches!(form.check, Check::Invalid(_)) {
        app.status = Some("corrija o campo destacado antes de enfileirar".into());
        return Vec::new();
    }
    let mut requests = Vec::new();
    for file in form.files.clone() {
        match form.request(&file) {
            Ok(r) => requests.push(r),
            Err((field, msg)) => {
                form.form.set_error(field, Some(msg.clone()));
                form.check = Check::Invalid(msg);
                return Vec::new();
            }
        }
    }
    form.submitting = true;
    vec![Effect::Enqueue { requests, reserved }]
}

fn on_enqueued(app: &mut App, results: Vec<(OpRequest, Result<PathBuf, String>)>) -> Vec<Effect> {
    let multiple = results.len() > 1;
    let mut errors = Vec::new();
    let mut added = 0;
    for (request, result) in results {
        match result {
            Ok(output) => {
                app.queue.push(request, output);
                added += 1;
            }
            Err(msg) => {
                let name = request
                    .input()
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                errors.push(if multiple {
                    format!("{name}: {msg}")
                } else {
                    msg
                });
            }
        }
    }
    if errors.is_empty() {
        close_form(app);
        app.status = Some(match added {
            1 => "1 job na fila (Tab para ver)".to_owned(),
            n => format!("{n} jobs na fila (Tab para ver)"),
        });
    } else if let Some(form) = app.form.as_mut() {
        form.submitting = false;
        let msg = errors.join("; ");
        let field = field_for_error(form, &msg);
        form.form.set_error(field, Some(msg.clone()));
        form.check = Check::Invalid(msg);
        if added > 0 {
            app.status = Some(format!(
                "{added} job(s) enfileirado(s); veja o erro dos demais"
            ));
        }
    }
    maybe_start(app)
}

// ---------------------------------------------------------------------------
// Corte: keyframes e miniatura

fn on_keyframes(app: &mut App, path: PathBuf, result: Result<Option<KeyframeIndex>, String>) {
    let Some(form) = app.form.as_mut() else {
        return;
    };
    if form.first_file() != path {
        return;
    }
    if let Some(cut) = form.cut.as_mut() {
        cut.keyframes = match result {
            Ok(Some(index)) => Keyframes::Ready(index),
            Ok(None) => Keyframes::NoVideo,
            Err(e) => Keyframes::Failed(e),
        };
    }
}

fn schedule_frame(app: &mut App, now: Instant) {
    let wants = app.thumbs.is_some()
        && app
            .form
            .as_ref()
            .and_then(|f| f.cut.as_ref())
            .is_some_and(|c| c.has_video());
    if wants {
        app.pending_frame = Some(now + FRAME_DELAY);
    }
}

/// Caixa da miniatura (células) e seu tamanho em pixels.
fn thumb_box(app: &App) -> Option<(Rect, u32, u32)> {
    let thumbs = app.thumbs.as_ref()?;
    let area = layout::thumb_inner(Rect::new(0, 0, app.size.0, app.size.1));
    if area.width < 4 || area.height < 2 {
        return None;
    }
    let w = u32::from(area.width) * u32::from(thumbs.font.0);
    let h = u32::from(area.height) * u32::from(thumbs.font.1);
    Some((area, w, h))
}

fn fire_frame(app: &mut App) -> Vec<Effect> {
    app.pending_frame = None;
    let Some((_, w, h)) = thumb_box(app) else {
        return Vec::new();
    };
    let Some(form) = app.form.as_mut() else {
        return Vec::new();
    };
    let Some(t) = form.edge(form.focused_edge()) else {
        return Vec::new();
    };
    let path = form.files[0].clone();
    let Some(cut) = form.cut.as_mut() else {
        return Vec::new();
    };
    if !cut.has_video() {
        return Vec::new();
    }
    let key: FrameKey = ((t * 1000.0).round() as u64, w, h);
    if cut.thumb.shown.as_ref().is_some_and(|s| s.key == key) {
        return Vec::new();
    }
    cut.thumb.seq += 1;
    if let Some(image) = cut.thumb.cache.get(key) {
        show_frame(app, key, image);
        return Vec::new();
    }
    cut.thumb.loading = true;
    vec![Effect::Frame {
        seq: cut.thumb.seq,
        path,
        t,
        key,
    }]
}

fn show_frame(app: &mut App, key: FrameKey, image: DynamicImage) {
    let area = thumb_box(app).map(|(a, _, _)| a);
    let picker = app.thumbs.as_ref().and_then(|t| t.picker.as_ref());
    let protocol = match (picker, area) {
        (Some(p), Some(area)) => p
            .new_protocol(image, area.as_size(), Resize::Fit(None))
            .ok(),
        _ => None,
    };
    if let Some(cut) = app.form.as_mut().and_then(|f| f.cut.as_mut()) {
        cut.thumb.shown = Some(Shown { key, protocol });
        cut.thumb.loading = false;
        cut.thumb.error = None;
    }
}

fn on_frame(app: &mut App, seq: u64, key: FrameKey, result: Result<DynamicImage, String>) {
    let Some(cut) = app.form.as_mut().and_then(|f| f.cut.as_mut()) else {
        return;
    };
    let current = cut.thumb.seq == seq;
    match result {
        Ok(image) => {
            cut.thumb.cache.insert(key, image.clone());
            if current {
                show_frame(app, key, image);
            }
        }
        Err(e) if current => {
            cut.thumb.loading = false;
            cut.thumb.error = Some(e);
        }
        Err(_) => {}
    }
}

// ---------------------------------------------------------------------------
// Fila

fn queue_key(app: &mut App, key: KeyEvent, _now: Instant) -> Vec<Effect> {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.queue.move_cursor(-1),
        KeyCode::Down | KeyCode::Char('j') => app.queue.move_cursor(1),
        KeyCode::Tab | KeyCode::Esc => app.screen = Screen::Browser,
        KeyCode::Char('q') => return request_quit(app),
        KeyCode::Char('x') => match app.queue.running() {
            Some(job) => {
                app.status = Some(format!("cancelando: {}", job.label));
                return vec![Effect::CancelJob(job.id)];
            }
            None => app.status = Some("nenhum job em execução".into()),
        },
        KeyCode::Char('d') => {
            if let Some(job) = app.queue.selected() {
                let (id, running) = (job.id, job.state.is_running());
                if running {
                    app.status = Some("o job está em execução: use x para cancelar".into());
                } else {
                    app.queue.remove(id);
                }
            }
        }
        _ => {}
    }
    Vec::new()
}

fn maybe_start(app: &mut App) -> Vec<Effect> {
    if app.quitting.is_some() {
        return Vec::new();
    }
    let Some(id) = app.queue.next_to_start() else {
        return Vec::new();
    };
    let reserved = app.queue.reservations(Some(id));
    let Some(job) = app.queue.get_mut(id) else {
        return Vec::new();
    };
    job.state = JobState::Preparing;
    vec![Effect::StartJob {
        id,
        request: job.request.clone(),
        reserved,
    }]
}

fn on_job_event(app: &mut App, id: JobId, event: JobEvent) {
    let Some(job) = app.queue.get_mut(id) else {
        return;
    };
    match event {
        JobEvent::Phase { name } => {
            job.state = JobState::Running {
                phase: name,
                progress: None,
            };
        }
        JobEvent::Progress(p) => {
            if let JobState::Running { progress, .. } = &mut job.state {
                *progress = Some(p);
            }
        }
        JobEvent::Warning(w) => {
            if !job.warnings.contains(&w) {
                job.warnings.push(w);
            }
        }
    }
}

fn on_job_finished(app: &mut App, id: JobId, result: Result<Report, JobFailure>) -> Vec<Effect> {
    if let Some(job) = app.queue.get_mut(id) {
        job.state = match result {
            Ok(report) => {
                job.output = Some(report.output.clone());
                JobState::Done(report)
            }
            Err(f) => match f.kind {
                FailKind::Discarded => JobState::Discarded(f.message),
                FailKind::Cancelled => JobState::Cancelled,
                FailKind::Failed => JobState::Failed {
                    message: f.message,
                    log_tail: f.log_tail,
                },
            },
        };
    }
    if let Some(code) = app.quitting {
        if app.queue.running().is_none() {
            return vec![Effect::Exit(code)];
        }
        return Vec::new();
    }
    maybe_start(app)
}

// ---------------------------------------------------------------------------

fn fire_due(app: &mut App, now: Instant) -> Vec<Effect> {
    let mut effects = Vec::new();
    if let Some((due, path)) = app.pending_probe.clone()
        && due <= now
    {
        app.pending_probe = None;
        effects.push(Effect::Probe(path));
    }
    if app.pending_prepare.is_some_and(|due| due <= now) {
        effects.extend(fire_prepare(app));
    }
    if app.pending_frame.is_some_and(|due| due <= now) {
        effects.extend(fire_frame(app));
    }
    effects
}
