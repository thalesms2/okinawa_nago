//! Navegador de arquivos (spec `interactive-mode` › Navegador de arquivos).

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use nago_core::container::Container;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// Extensões de mídia além dos containers que o nago grava.
const EXTRA_MEDIA: &[&str] = &[
    "avi", "wmv", "flv", "ts", "m2ts", "mts", "mpg", "mpeg", "vob", "3gp", "ogv", "wav", "aac",
    "ogg", "oga", "wma", "aiff", "aif", "ac3", "eac3", "dts", "alac", "mka",
];

pub fn is_media(name: &str) -> bool {
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    Container::parse(&ext).is_some() || EXTRA_MEDIA.contains(&ext.as_str())
}

/// Ocultos sempre: arquivos ponto, o que inclui os temporários do nago
/// (`.<nome>.nago-XXXX.part.<ext>`).
pub fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Ordem natural: `video2` antes de `video10`, sem diferenciar maiúsculas.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Pastas primeiro, depois ordem natural.
pub fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|x, y| {
        y.is_dir
            .cmp(&x.is_dir)
            .then_with(|| natural_cmp(&x.name, &y.name))
    });
}

/// Lê o diretório, já sem ocultos e ordenado.
pub fn read_dir(dir: &Path) -> std::io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        if is_hidden(&name) {
            continue;
        }
        let path = e.path();
        // segue symlinks: um link para pasta é navegável
        let is_dir = path.is_dir();
        entries.push(Entry { name, path, is_dir });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

#[derive(Debug, Clone, Default)]
pub struct Browser {
    pub dir: PathBuf,
    /// Conteúdo do diretório (sem filtro de extensão).
    pub entries: Vec<Entry>,
    pub show_all: bool,
    pub cursor: usize,
    /// Arquivos marcados (podem estar em outras pastas).
    pub marked: BTreeSet<PathBuf>,
    pub loading: bool,
    pub error: Option<String>,
    /// Arquivo a destacar quando a listagem chegar.
    pub select_on_load: Option<PathBuf>,
}

impl Browser {
    pub fn new(dir: PathBuf) -> Browser {
        Browser {
            dir,
            loading: true,
            ..Browser::default()
        }
    }

    fn parent_entry(&self) -> Option<Entry> {
        self.dir.parent().map(|p| Entry {
            name: "..".into(),
            path: p.to_path_buf(),
            is_dir: true,
        })
    }

    /// Linhas exibidas: `..` (se houver pai), pastas e arquivos de mídia (ou todos).
    pub fn visible(&self) -> Vec<Entry> {
        self.parent_entry()
            .into_iter()
            .chain(
                self.entries
                    .iter()
                    .filter(|e| e.is_dir || self.show_all || is_media(&e.name))
                    .cloned(),
            )
            .collect()
    }

    pub fn selected(&self) -> Option<Entry> {
        self.visible().into_iter().nth(self.cursor)
    }

    /// Arquivo destacado (não pasta).
    pub fn selected_file(&self) -> Option<PathBuf> {
        self.selected().filter(|e| !e.is_dir).map(|e| e.path)
    }

    /// Alvos de uma operação: os marcados ou, sem marcação, o arquivo destacado.
    pub fn targets(&self) -> Vec<PathBuf> {
        if self.marked.is_empty() {
            self.selected_file().into_iter().collect()
        } else {
            self.marked.iter().cloned().collect()
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.visible().len();
        if n == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = self.cursor.saturating_add_signed(delta).min(n - 1);
    }

    pub fn toggle_mark(&mut self) {
        if let Some(path) = self.selected_file()
            && !self.marked.remove(&path)
        {
            self.marked.insert(path);
        }
    }

    /// Instala uma listagem nova, mantendo o destaque no arquivo pedido (se houver).
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.loading = false;
        self.error = None;
        // começa no primeiro item depois de `..`
        self.cursor = usize::from(self.parent_entry().is_some() && self.visible().len() > 1);
        if let Some(target) = self.select_on_load.take()
            && let Some(i) = self.visible().iter().position(|e| e.path == target)
        {
            self.cursor = i;
        }
        self.move_cursor(0);
    }

    /// Alterna o filtro mantendo o destaque na mesma entrada quando ela continua visível.
    pub fn toggle_show_all(&mut self) {
        let current = self.selected().map(|e| e.path);
        self.show_all = !self.show_all;
        self.cursor = current
            .and_then(|p| self.visible().iter().position(|e| e.path == p))
            .unwrap_or(0);
        self.move_cursor(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.into(),
            path: PathBuf::from("/v").join(name),
            is_dir,
        }
    }

    #[test]
    fn ordem_natural_com_pastas_primeiro() {
        let mut v = vec![
            entry("video10.mp4", false),
            entry("Video2.mp4", false),
            entry("b", true),
            entry("a", true),
            entry("video1.mp4", false),
        ];
        sort_entries(&mut v);
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "video1.mp4", "Video2.mp4", "video10.mp4"]);
    }

    #[test]
    fn extensoes_de_midia() {
        assert!(is_media("video.MP4"));
        assert!(is_media("musica.flac"));
        assert!(is_media("antigo.avi"));
        assert!(!is_media("notas.txt"));
        assert!(!is_media("sem_extensao"));
        assert!(is_hidden(".video.nago-ab12.part.mkv"));
    }

    #[test]
    fn filtro_de_extensoes() {
        let mut b = Browser::new("/v".into());
        b.set_entries(vec![
            entry("musica.flac", false),
            entry("notas.txt", false),
            entry("video.mp4", false),
        ]);
        let names =
            |b: &Browser| -> Vec<String> { b.visible().into_iter().map(|e| e.name).collect() };
        assert_eq!(names(&b), ["..", "musica.flac", "video.mp4"]);
        b.toggle_show_all();
        assert_eq!(names(&b), ["..", "musica.flac", "notas.txt", "video.mp4"]);
    }

    #[test]
    fn alvos_sao_os_marcados_ou_o_destacado() {
        let mut b = Browser::new("/v".into());
        b.set_entries(vec![entry("a.mp4", false), entry("b.mp4", false)]);
        assert_eq!(b.cursor, 1, "começa no primeiro item depois de `..`");
        assert_eq!(b.targets(), [PathBuf::from("/v/a.mp4")]);
        b.toggle_mark();
        b.move_cursor(1);
        b.toggle_mark();
        assert_eq!(b.targets().len(), 2);
        b.toggle_mark();
        assert_eq!(b.targets(), [PathBuf::from("/v/a.mp4")]);
    }
}
