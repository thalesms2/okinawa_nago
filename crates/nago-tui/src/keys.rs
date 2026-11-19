//! Atalhos de teclado por tela: fonte única da ajuda (`?`) e da documentação.

/// Telas com conjunto próprio de atalhos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpScreen {
    Browser,
    Form,
    Cut,
    Queue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub keys: &'static str,
    pub action: &'static str,
}

const fn s(keys: &'static str, action: &'static str) -> Shortcut {
    Shortcut { keys, action }
}

const GLOBAL: &[Shortcut] = &[
    s("?", "mostrar/ocultar esta ajuda"),
    s("Ctrl+C", "sair (duas vezes seguidas confirma)"),
];

const BROWSER: &[Shortcut] = &[
    s("↑/↓  k/j", "navegar"),
    s("Enter  l", "entrar na pasta"),
    s("Backspace  h", "pasta pai"),
    s("Espaço", "marcar/desmarcar arquivo"),
    s(".", "mostrar todos os arquivos / só mídia"),
    s("c", "cortar"),
    s("v", "converter"),
    s("z", "comprimir"),
    s("Tab", "ver a fila"),
    s("q", "sair"),
];

const FORM: &[Shortcut] = &[
    s("↑/↓  Tab/Shift+Tab", "campo anterior/seguinte"),
    s("←/→  Espaço", "mudar opção / alternar"),
    s("Backspace  Ctrl+U", "apagar caractere / limpar campo"),
    s("Enter", "enfileirar"),
    s("Esc", "voltar ao navegador"),
];

const CUT: &[Shortcut] = &[
    s("i / o", "editar início / fim"),
    s("< / >", "1 frame para trás / para frente"),
    s("←/→", "1 s para trás / para frente"),
    s("Shift+←/→", "10 s para trás / para frente"),
    s("[ / ]", "keyframe anterior / seguinte"),
    s("Enter", "enfileirar"),
    s("Esc", "voltar ao navegador"),
];

const QUEUE: &[Shortcut] = &[
    s("↑/↓  k/j", "selecionar job"),
    s("x", "cancelar o job em execução"),
    s("d", "remover o job selecionado (pendente ou terminado)"),
    s("Tab  Esc", "voltar ao navegador"),
    s("q", "sair"),
];

/// Atalhos específicos da tela, seguidos dos globais.
pub fn for_screen(screen: HelpScreen) -> Vec<Shortcut> {
    let own = match screen {
        HelpScreen::Browser => BROWSER,
        HelpScreen::Form => FORM,
        HelpScreen::Cut => CUT,
        HelpScreen::Queue => QUEUE,
    };
    own.iter().chain(GLOBAL).copied().collect()
}
