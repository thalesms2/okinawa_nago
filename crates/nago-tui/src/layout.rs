//! Divisão da tela, compartilhada pela `view` e pelo `update` (que precisa do tamanho
//! da miniatura em pixels para pedir o frame certo).

use ratatui::layout::{Constraint, Layout, Rect};

/// Cabeçalho (1 linha), corpo e rodapé (1 linha).
pub fn main(area: Rect) -> (Rect, Rect, Rect) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);
    (header, body, footer)
}

/// Lista de arquivos e painel de metadados.
pub fn browser(body: Rect) -> (Rect, Rect) {
    let [list, info] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(body);
    (list, info)
}

/// Colunas do formulário: campos/resumo à esquerda, painel lateral à direita.
pub fn form(body: Rect) -> (Rect, Rect) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(body);
    (left, right)
}

/// Painel lateral do corte: miniatura em cima, informações do trecho embaixo.
pub fn cut_side(right: Rect) -> (Rect, Rect) {
    let [thumb, info] =
        Layout::vertical([Constraint::Percentage(65), Constraint::Percentage(35)]).areas(right);
    (thumb, info)
}

/// Área interna (sem bordas) do quadro da miniatura, para uma tela `screen`.
pub fn thumb_inner(screen: Rect) -> Rect {
    let (_, body, _) = main(screen);
    let (_, right) = form(body);
    let (thumb, _) = cut_side(right);
    inner(thumb)
}

/// Área sem a borda de um `Block` com bordas em volta.
pub fn inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// Retângulo centralizado para janelas sobrepostas.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}
