//! Desenho das telas a partir do estado (sem efeitos colaterais).

use nago_core::display::{self, Tone};
use nago_core::timefmt;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Wrap};
use ratatui_image::Image;

use crate::app::{App, InfoState, Overlay, Screen};
use crate::form::{FieldKind, Form};
use crate::keys;
use crate::layout;
use crate::opform::{Check, Keyframes, OpForm, OpKind};
use crate::queue::{Job, JobState};

fn tone_style(tone: Tone) -> Style {
    match tone {
        Tone::Normal => Style::new(),
        Tone::Label => Style::new().add_modifier(Modifier::BOLD),
        Tone::Path => Style::new().fg(Color::Cyan),
        Tone::Good => Style::new().fg(Color::Green),
        Tone::Bad => Style::new().fg(Color::Red),
        Tone::Muted => Style::new().fg(Color::DarkGray),
    }
}

fn lines_of(lines: &[display::Line]) -> Vec<Line<'static>> {
    lines
        .iter()
        .map(|l| {
            Line::from(
                l.0.iter()
                    .map(|s| Span::styled(s.text.clone(), tone_style(s.tone)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn block(title: &str) -> Block<'_> {
    Block::new()
        .borders(Borders::ALL)
        .title(Span::styled(format!(" {title} "), Style::new().bold()))
}

fn warning(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("⚠ {text}"),
        Style::new().fg(Color::Yellow),
    ))
}

pub fn draw(app: &App, frame: &mut Frame) {
    let (header, body, footer) = layout::main(frame.area());
    draw_header(app, frame, header);
    match app.screen {
        Screen::Browser => draw_browser(app, frame, body),
        Screen::Form => match &app.form {
            Some(form) => draw_form(app, form, frame, body),
            None => draw_browser(app, frame, body),
        },
        Screen::Queue => draw_queue(app, frame, body),
    }
    draw_footer(app, frame, footer);
    match app.overlay {
        Some(Overlay::Help) => draw_help(app, frame),
        Some(Overlay::ConfirmQuit) => draw_confirm(app, frame),
        None => {}
    }
}

fn draw_header(app: &App, frame: &mut Frame, area: Rect) {
    let where_ = match app.screen {
        Screen::Browser => app.browser.dir.display().to_string(),
        Screen::Form => app.form.as_ref().map(form_title).unwrap_or_default(),
        Screen::Queue => "fila".to_owned(),
    };
    let q = &app.queue;
    let running = q.running().map_or(0, |_| 1);
    let queue = format!(
        "fila: {running} em execução · {} pendente(s) · {} total ",
        q.pending_count(),
        q.jobs.len()
    );
    let queue_width = queue.chars().count() as u16;
    let [left_area, right_area] = ratatui::layout::Layout::horizontal([
        ratatui::layout::Constraint::Min(10),
        ratatui::layout::Constraint::Length(queue_width.min(area.width / 2)),
    ])
    .areas(area);
    // caminhos longos perdem o começo, que é o menos informativo
    let room = (left_area.width as usize).saturating_sub(8);
    let count = where_.chars().count();
    let where_ = if count > room {
        let tail: String = where_
            .chars()
            .skip(count - room.saturating_sub(1))
            .collect();
        format!("…{tail}")
    } else {
        where_
    };
    let left = Line::from(vec![
        Span::styled(" nago ", Style::new().reversed().bold()),
        Span::raw(" "),
        Span::raw(where_),
    ]);
    frame.render_widget(Paragraph::new(left), left_area);
    frame.render_widget(
        Paragraph::new(Span::styled(queue, Style::new().fg(Color::DarkGray)))
            .alignment(Alignment::Right),
        right_area,
    );
}

fn draw_footer(app: &App, frame: &mut Frame, area: Rect) {
    let text = match &app.status {
        Some(s) => Line::from(Span::styled(
            format!(" {s}"),
            Style::new().fg(Color::Yellow),
        )),
        None => {
            let hint = match app.screen {
                Screen::Browser => {
                    " c cortar · v converter · z comprimir · Espaço marcar · Tab fila · ? ajuda · q sair"
                }
                Screen::Form => " Enter enfileirar · Esc voltar · ? ajuda",
                Screen::Queue => " x cancelar · d remover · Tab voltar · ? ajuda · q sair",
            };
            Line::from(Span::styled(hint, Style::new().fg(Color::DarkGray)))
        }
    };
    frame.render_widget(Paragraph::new(text), area);
}

// ---------------------------------------------------------------------------
// Navegador

fn draw_browser(app: &App, frame: &mut Frame, body: Rect) {
    let (list_area, info_area) = layout::browser(body);
    let b = &app.browser;
    let title = if b.show_all {
        "arquivos (todos)"
    } else {
        "arquivos de mídia"
    };
    let entries = b.visible();
    let items: Vec<ListItem> = entries
        .iter()
        .map(|e| {
            let mark = if b.marked.contains(&e.path) {
                "[x] "
            } else if e.is_dir {
                "    "
            } else {
                "[ ] "
            };
            let name = if e.is_dir {
                Span::styled(format!("{}/", e.name), Style::new().fg(Color::Blue).bold())
            } else {
                Span::raw(e.name.clone())
            };
            ListItem::new(Line::from(vec![Span::raw(mark), name]))
        })
        .collect();
    let list_block = block(title);
    if b.loading {
        frame.render_widget(
            Paragraph::new("lendo a pasta…").block(list_block),
            list_area,
        );
    } else if let Some(err) = &b.error {
        frame.render_widget(
            Paragraph::new(Span::styled(err.clone(), Style::new().fg(Color::Red)))
                .wrap(Wrap { trim: false })
                .block(list_block),
            list_area,
        );
    } else {
        let list = List::new(items)
            .block(list_block)
            .highlight_style(Style::new().reversed());
        let mut state = ListState::default().with_selected(Some(b.cursor));
        frame.render_stateful_widget(list, list_area, &mut state);
    }

    let info_block = block("informações");
    let content: Text = match b.selected() {
        Some(e) if e.is_dir => Text::from(Span::styled(
            format!("pasta {}", e.name),
            Style::new().fg(Color::DarkGray),
        )),
        Some(e) => info_text(app, &e.path),
        None => Text::from(Span::styled(
            "nenhum arquivo de mídia nesta pasta ( . mostra todos )",
            Style::new().fg(Color::DarkGray),
        )),
    };
    frame.render_widget(
        Paragraph::new(content)
            .wrap(Wrap { trim: false })
            .block(info_block),
        info_area,
    );
}

fn info_text(app: &App, path: &std::path::Path) -> Text<'static> {
    match app.info(path) {
        Some(InfoState::Ready(info)) => {
            let mut lines = lines_of(&display::media_info(info));
            if let Some(first) = lines.first_mut() {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                *first = Line::from(Span::styled(name, tone_style(Tone::Path).bold()));
            }
            Text::from(lines)
        }
        Some(InfoState::Failed(e)) => Text::from(vec![
            Line::from(Span::styled(
                "não foi possível ler este arquivo:",
                Style::new().fg(Color::Red).bold(),
            )),
            Line::from(Span::styled(e.clone(), Style::new().fg(Color::Red))),
        ]),
        _ => Text::from(Span::styled("lendo…", Style::new().fg(Color::DarkGray))),
    }
}

// ---------------------------------------------------------------------------
// Formulários

fn form_title(f: &OpForm) -> String {
    let name = f
        .first_file()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extra = match f.files.len() {
        1 => String::new(),
        n => format!(" (+{} arquivos)", n - 1),
    };
    let verb = match f.kind {
        OpKind::Cut => "Cortar",
        OpKind::Convert => "Converter",
        OpKind::Compress => "Comprimir",
    };
    format!("{verb} {name}{extra}")
}

fn field_lines(form: &Form) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (i, f) in form.fields.iter().enumerate() {
        let focused = i == form.focus;
        let value: Span = match &f.kind {
            FieldKind::Text {
                value, placeholder, ..
            } => {
                if value.is_empty() && !focused {
                    Span::styled(placeholder.to_string(), Style::new().fg(Color::DarkGray))
                } else if focused {
                    Span::raw(format!("{value}▏"))
                } else {
                    Span::raw(value.clone())
                }
            }
            FieldKind::Choice { options, idx } => Span::raw(format!("◂ {} ▸", options[*idx])),
            FieldKind::Toggle(on) => Span::raw(if *on { "[x] sim" } else { "[ ] não" }),
        };
        let label_style = if focused {
            Style::new().reversed().bold()
        } else {
            Style::new().bold()
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {:<16}", f.label), label_style),
            Span::raw(" "),
            value,
        ]));
        if let Some(err) = &f.error {
            lines.push(Line::from(Span::styled(
                format!("   ✗ {err}"),
                Style::new().fg(Color::Red),
            )));
        }
    }
    lines
}

fn summary_lines(form: &OpForm) -> Vec<Line<'static>> {
    match &form.check {
        Check::Checking => vec![Line::from(Span::styled(
            "verificando…",
            Style::new().fg(Color::DarkGray),
        ))],
        Check::Invalid(msg) => vec![Line::from(Span::styled(
            format!("✗ {msg}"),
            Style::new().fg(Color::Red),
        ))],
        Check::Ready(s) => {
            let mut lines = vec![Line::from(vec![
                Span::styled("saída: ", Style::new().bold()),
                Span::styled(s.output.display().to_string(), tone_style(Tone::Path)),
            ])];
            if form.files.len() > 1 {
                lines.push(Line::from(Span::styled(
                    format!(
                        "(e uma saída equivalente para cada um dos {} arquivos)",
                        form.files.len()
                    ),
                    Style::new().fg(Color::DarkGray),
                )));
            }
            let encoder = match (&s.encoder, s.quality) {
                (Some(e), Some(q)) => format!("{e} · perfil {}", q.name()),
                (Some(e), None) => e.clone(),
                (None, _) => "nenhum (vídeo copiado)".to_owned(),
            };
            lines.push(Line::from(vec![
                Span::styled("encoder: ", Style::new().bold()),
                Span::raw(encoder),
            ]));
            lines.push(Line::from(vec![
                Span::styled("streams: ", Style::new().bold()),
                Span::raw(s.streams.join(" · ")),
            ]));
            lines.extend(s.warnings.iter().map(|w| warning(w)));
            lines
        }
    }
}

fn draw_form(app: &App, form: &OpForm, frame: &mut Frame, body: Rect) {
    let (left, right) = layout::form(body);
    let fields = field_lines(&form.form);
    let field_height = (fields.len() as u16 + 2).min(left.height.saturating_sub(4));
    let [fields_area, summary_area] = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(field_height),
        ratatui::layout::Constraint::Min(3),
    ])
    .areas(left);
    frame.render_widget(
        Paragraph::new(fields).block(block(&form_title(form))),
        fields_area,
    );
    let summary_title = if form.submitting {
        "enfileirando…"
    } else {
        "resultado previsto"
    };
    frame.render_widget(
        Paragraph::new(summary_lines(form))
            .wrap(Wrap { trim: false })
            .block(block(summary_title)),
        summary_area,
    );

    if form.kind == OpKind::Cut && form.cut.is_some() {
        draw_cut_side(app, form, frame, right);
    } else {
        frame.render_widget(
            Paragraph::new(info_text(app, form.first_file()))
                .wrap(Wrap { trim: false })
                .block(block("informações")),
            right,
        );
    }
}

fn draw_cut_side(app: &App, form: &OpForm, frame: &mut Frame, right: Rect) {
    let Some(cut) = &form.cut else {
        return;
    };
    let info_area = if app.thumbs.is_some() && cut.has_video() {
        let (thumb_area, info_area) = layout::cut_side(right);
        let edge = match form.focused_edge() {
            crate::form::FieldId::End => "fim",
            _ => "início",
        };
        let title = match form.edge(form.focused_edge()) {
            Some(t) => format!("frame do {edge} · {}", timefmt::format(t)),
            None => format!("frame do {edge}"),
        };
        frame.render_widget(block(&title), thumb_area);
        let inner = layout::inner(thumb_area);
        match cut.thumb.shown.as_ref().and_then(|s| s.protocol.as_ref()) {
            Some(protocol) => frame.render_widget(Image::new(protocol), inner),
            None => {
                let msg = match (&cut.thumb.error, cut.thumb.loading) {
                    (Some(e), _) => Span::styled(e.clone(), Style::new().fg(Color::Red)),
                    (None, true) => Span::styled("carregando…", Style::new().fg(Color::DarkGray)),
                    (None, false) => Span::raw(""),
                };
                frame.render_widget(Paragraph::new(msg).wrap(Wrap { trim: false }), inner);
            }
        }
        if cut.thumb.loading && cut.thumb.shown.is_some() {
            let tag = Rect {
                x: inner.x,
                y: inner.y + inner.height.saturating_sub(1),
                width: inner.width.min(14),
                height: 1.min(inner.height),
            };
            frame.render_widget(
                Paragraph::new(Span::styled(" carregando… ", Style::new().reversed())),
                tag,
            );
        }
        info_area
    } else {
        right
    };

    let mut lines = Vec::new();
    let start = form.edge(crate::form::FieldId::Start);
    let end = form.edge(crate::form::FieldId::End);
    if let (Some(s), Some(e)) = (start, end) {
        lines.push(Line::from(vec![
            Span::styled("trecho: ", Style::new().bold()),
            Span::raw(format!(
                "{} → {} ({})",
                timefmt::format(s),
                timefmt::format(e),
                display::duration((e - s).max(0.0))
            )),
        ]));
    }
    lines.push(Line::from(vec![
        Span::styled("arquivo: ", Style::new().bold()),
        Span::raw(timefmt::format(cut.duration)),
    ]));
    match &cut.keyframes {
        Keyframes::NoVideo => lines.push(Line::from(Span::styled(
            "arquivo sem vídeo: ajuste por tempo (←/→, Shift+←/→)",
            Style::new().fg(Color::DarkGray),
        ))),
        Keyframes::Indexing => lines.push(Line::from(Span::styled(
            "indexando keyframes…",
            Style::new().fg(Color::DarkGray),
        ))),
        Keyframes::Failed(e) => lines.push(Line::from(Span::styled(
            format!("keyframes indisponíveis: {e}"),
            Style::new().fg(Color::Red),
        ))),
        Keyframes::Ready(index) => {
            lines.push(Line::from(Span::styled(
                format!("{} keyframes ( [ / ] para saltar )", index.len()),
                Style::new().fg(Color::DarkGray),
            )));
            if form.form.toggle(crate::form::FieldId::Exact) {
                lines.push(Line::from(
                    "modo exato: recodifica o trecho e começa no frame pedido",
                ));
            } else if let Some((kf, diff)) = form.fast_cut_start() {
                if diff > 0.001 {
                    lines.push(Line::from(format!(
                        "corte rápido começa no keyframe {} ({:.1} s antes)",
                        timefmt::format(kf),
                        diff
                    )));
                } else {
                    lines.push(Line::from(
                        "o início pedido é um keyframe: corte rápido exato",
                    ));
                }
                if diff > 1.0 {
                    lines.push(warning(
                        "diferença maior que 1 s: considere o modo exato ao frame",
                    ));
                }
            }
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block("corte")),
        info_area,
    );
}

// ---------------------------------------------------------------------------
// Fila

fn job_line(job: &Job) -> Line<'static> {
    let (icon, style) = match &job.state {
        JobState::Pending => ("·", Style::new().fg(Color::DarkGray)),
        JobState::Preparing | JobState::Running { .. } => ("▶", Style::new().fg(Color::Cyan)),
        JobState::Done(_) => ("✓", Style::new().fg(Color::Green)),
        JobState::Discarded(_) => ("=", Style::new().fg(Color::Yellow)),
        JobState::Failed { .. } => ("✗", Style::new().fg(Color::Red)),
        JobState::Cancelled => ("■", Style::new().fg(Color::DarkGray)),
    };
    let mut status = job.state.label().to_owned();
    if let JobState::Running {
        progress: Some(p), ..
    } = &job.state
    {
        status = format!("{:.0}%", p.ratio * 100.0);
    }
    Line::from(vec![
        Span::styled(format!(" {icon} "), style),
        Span::raw(job.label.clone()),
        Span::styled(format!("  {status}"), style),
    ])
}

fn draw_queue(app: &App, frame: &mut Frame, body: Rect) {
    let (list_area, detail_area) = layout::browser(body);
    let q = &app.queue;
    if q.jobs.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "nenhum job: escolha um arquivo e uma operação no navegador (Tab)",
                Style::new().fg(Color::DarkGray),
            ))
            .wrap(Wrap { trim: false })
            .block(block("fila")),
            body,
        );
        return;
    }
    let items: Vec<ListItem> = q.jobs.iter().map(|j| ListItem::new(job_line(j))).collect();
    let mut state = ListState::default().with_selected(Some(q.cursor));
    frame.render_stateful_widget(
        List::new(items)
            .block(block("fila"))
            .highlight_style(Style::new().reversed()),
        list_area,
        &mut state,
    );
    if let Some(job) = q.selected() {
        draw_job_detail(job, frame, detail_area);
    }
}

fn draw_job_detail(job: &Job, frame: &mut Frame, area: Rect) {
    let outer = block(&job.label);
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let output = job.output.as_ref().unwrap_or(&job.planned_output);
    let mut lines: Vec<Line> = Vec::new();
    let mut gauge: Option<(f64, String)> = None;
    match &job.state {
        JobState::Pending => {
            lines.push(Line::from("aguardando a vez na fila"));
            lines.push(Line::from(vec![
                Span::styled("saída prevista: ", Style::new().bold()),
                Span::styled(output.display().to_string(), tone_style(Tone::Path)),
            ]));
        }
        JobState::Preparing => lines.push(Line::from("preparando…")),
        JobState::Running { phase, progress } => {
            lines.push(Line::from(vec![
                Span::styled("saída: ", Style::new().bold()),
                Span::styled(output.display().to_string(), tone_style(Tone::Path)),
            ]));
            let mut detail = Vec::new();
            if let Some(p) = progress {
                if let Some(s) = p.speed {
                    detail.push(format!("{s:.1}x"));
                }
                if let Some(eta) = p.eta_secs {
                    detail.push(format!("faltam {}", display::duration(eta)));
                }
                gauge = Some((p.ratio.clamp(0.0, 1.0), format!("{:.0}%", p.ratio * 100.0)));
            }
            lines.push(Line::from(vec![
                Span::styled("fase: ", Style::new().bold()),
                Span::raw(if phase.is_empty() { "iniciando" } else { phase }.to_owned()),
                Span::raw(if detail.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", detail.join(" · "))
                }),
            ]));
        }
        JobState::Done(report) => lines.extend(lines_of(&display::report(report))),
        JobState::Discarded(msg) => {
            lines.push(Line::from(Span::styled(
                "descartado: a saída não seria menor que a entrada",
                Style::new().fg(Color::Yellow).bold(),
            )));
            lines.push(Line::from(msg.clone()));
        }
        JobState::Failed { message, log_tail } => {
            lines.push(Line::from(Span::styled(
                format!("✗ {message}"),
                Style::new().fg(Color::Red),
            )));
            if !log_tail.is_empty() {
                lines.push(Line::from(Span::styled(
                    "últimas linhas do ffmpeg:",
                    Style::new().bold(),
                )));
                lines.extend(log_tail.iter().map(|l| {
                    Line::from(Span::styled(
                        format!("  {l}"),
                        Style::new().fg(Color::DarkGray),
                    ))
                }));
            }
        }
        JobState::Cancelled => lines.push(Line::from(Span::styled(
            "cancelado: nenhum arquivo parcial foi mantido",
            Style::new().fg(Color::DarkGray),
        ))),
    }
    let report_warnings: &[String] = match &job.state {
        JobState::Done(r) => &r.warnings,
        _ => &[],
    };
    for w in job.warnings.iter().chain(report_warnings) {
        let line = warning(w);
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    let (text_area, gauge_area) = match gauge {
        Some(_) if inner.height > 2 => {
            let [g, t] = ratatui::layout::Layout::vertical([
                ratatui::layout::Constraint::Length(1),
                ratatui::layout::Constraint::Min(1),
            ])
            .areas(inner);
            (t, Some(g))
        }
        _ => (inner, None),
    };
    if let (Some((ratio, label)), Some(g)) = (gauge, gauge_area) {
        frame.render_widget(
            Gauge::default()
                .ratio(ratio)
                .label(label)
                .gauge_style(Style::new().fg(Color::Cyan)),
            g,
        );
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), text_area);
}

// ---------------------------------------------------------------------------
// Janelas sobrepostas

fn draw_help(app: &App, frame: &mut Frame) {
    let shortcuts = keys::for_screen(app.help_screen());
    const KEYS_COL: usize = 21;
    let width = shortcuts
        .iter()
        .map(|s| KEYS_COL + s.action.chars().count() + 1)
        .max()
        .unwrap_or(30) as u16
        + 2;
    let area = layout::centered(frame.area(), width, shortcuts.len() as u16 + 2);
    let lines: Vec<Line> = shortcuts
        .iter()
        .map(|s| {
            Line::from(vec![
                Span::styled(format!(" {:<20}", s.keys), Style::new().bold()),
                Span::raw(s.action),
            ])
        })
        .collect();
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(block("atalhos")), area);
}

fn draw_confirm(app: &App, frame: &mut Frame) {
    let running = app.queue.running().is_some();
    let pending = app.queue.pending_count();
    let mut lines = vec![
        Line::from(Span::styled("Sair agora?", Style::new().bold())),
        Line::from(""),
    ];
    if running {
        lines.push(Line::from(
            "• o job em execução será cancelado (sem deixar arquivo parcial)",
        ));
    }
    if pending > 0 {
        lines.push(Line::from(format!(
            "• {pending} job(s) pendente(s) serão descartados"
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "s / Enter: sair    n / Esc: continuar",
        Style::new().bold(),
    )));
    let height = lines.len() as u16 + 2;
    let area = layout::centered(frame.area(), 70, height);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block("confirmar saída")),
        area,
    );
}
