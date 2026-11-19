//! Resolução do caminho de saída (spec `job-execution` › Proteção de arquivos existentes).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Como nomear a saída quando o usuário não passa `-o`.
#[derive(Debug, Clone, Copy)]
pub struct DefaultName<'a> {
    /// Sufixo inserido antes da extensão (`cut` → `aula.cut.mp4`); `None` → `aula.mp4`.
    pub suffix: Option<&'a str>,
    /// Extensão final, sem ponto.
    pub ext: &'a str,
}

/// Caminhos de saída já prometidos a outros jobs (ex.: pendentes na fila da TUI).
///
/// Um caminho reservado conta como ocupado mesmo que ainda não exista no disco.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reservations(BTreeSet<PathBuf>);

impl Reservations {
    pub fn new() -> Reservations {
        Reservations::default()
    }

    pub fn insert(&mut self, path: &Path) {
        self.0.insert(key(path));
    }

    pub fn remove(&mut self, path: &Path) {
        self.0.remove(&key(path));
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.0.contains(&key(path))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Chave de comparação: caminho canônico quando possível.
fn key(path: &Path) -> PathBuf {
    canonical(path).unwrap_or_else(|| path.to_path_buf())
}

/// Resolve o caminho final da saída.
///
/// - `explicit` (`-o`): usado como está; se for um diretório existente, o nome padrão
///   é criado dentro dele. Se o arquivo existir, exige `overwrite`.
/// - sem `-o`: `<dir da entrada>/<nome>[.<sufixo>].<ext>`, acrescentando `-1`, `-2`...
///   em caso de colisão.
/// - em nenhum caso a saída pode ser a própria entrada (nem via symlink).
/// - caminhos em `reserved` contam como ocupados: o nome padrão os pula e um `-o`
///   reservado é recusado, mesmo com `overwrite`.
pub fn resolve(
    input: &Path,
    explicit: Option<&Path>,
    default: DefaultName<'_>,
    overwrite: bool,
    reserved: &Reservations,
) -> Result<PathBuf> {
    let output = match explicit {
        Some(path) if !path.is_dir() => {
            ensure_not_input(input, path)?;
            if reserved.contains(path) {
                return Err(Error::usage(format!(
                    "outro job da fila já vai gravar em {}; escolha outro caminho",
                    path.display()
                )));
            }
            if path.exists() && !overwrite {
                return Err(Error::usage(format!(
                    "{} já existe; use --overwrite para substituí-lo ou escolha outro -o",
                    path.display()
                )));
            }
            path.to_path_buf()
        }
        Some(dir) => free_default_name(input, dir, default, reserved),
        None => {
            let dir = input
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            free_default_name(input, dir, default, reserved)
        }
    };
    ensure_not_input(input, &output)?;
    Ok(output)
}

fn free_default_name(
    input: &Path,
    dir: &Path,
    default: DefaultName<'_>,
    reserved: &Reservations,
) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "saida".to_owned());
    let base = match default.suffix {
        Some(suffix) => format!("{stem}.{suffix}"),
        None => stem,
    };
    let candidate = |n: u32| {
        let name = if n == 0 {
            format!("{base}.{}", default.ext)
        } else {
            format!("{base}-{n}.{}", default.ext)
        };
        dir.join(name)
    };
    (0..)
        .map(candidate)
        .find(|p| !p.exists() && !reserved.contains(p) && !same_file(input, p))
        .expect("sempre existe um nome livre")
}

fn ensure_not_input(input: &Path, output: &Path) -> Result<()> {
    if same_file(input, output) {
        return Err(Error::usage(format!(
            "a saída {} é o próprio arquivo de entrada; o nago nunca sobrescreve a entrada",
            output.display()
        )));
    }
    Ok(())
}

/// Compara caminhos resolvendo symlinks; para saídas inexistentes, resolve o diretório pai.
fn same_file(a: &Path, b: &Path) -> bool {
    match (canonical(a), canonical(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

fn canonical(p: &Path) -> Option<PathBuf> {
    if let Ok(c) = p.canonicalize() {
        return Some(c);
    }
    let parent = p
        .parent()
        .filter(|x| !x.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Some(parent.canonicalize().ok()?.join(p.file_name()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUT: DefaultName = DefaultName {
        suffix: Some("cut"),
        ext: "mp4",
    };

    const NONE: Reservations = Reservations(BTreeSet::new());

    fn setup() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("aula.mp4");
        std::fs::write(&input, b"x").unwrap();
        (dir, input)
    }

    #[test]
    fn nome_padrao() {
        let (dir, input) = setup();
        let out = resolve(&input, None, CUT, false, &NONE).unwrap();
        assert_eq!(out, dir.path().join("aula.cut.mp4"));
    }

    #[test]
    fn nome_padrao_com_colisao_ganha_sufixo_numerico() {
        let (dir, input) = setup();
        std::fs::write(dir.path().join("aula.cut.mp4"), b"x").unwrap();
        assert_eq!(
            resolve(&input, None, CUT, false, &NONE).unwrap(),
            dir.path().join("aula.cut-1.mp4")
        );
        std::fs::write(dir.path().join("aula.cut-1.mp4"), b"x").unwrap();
        assert_eq!(
            resolve(&input, None, CUT, false, &NONE).unwrap(),
            dir.path().join("aula.cut-2.mp4")
        );
    }

    #[test]
    fn nome_padrao_sem_sufixo_igual_a_entrada_ganha_contador() {
        let (dir, input) = setup();
        let conv = DefaultName {
            suffix: None,
            ext: "mp4",
        };
        assert_eq!(
            resolve(&input, None, conv, false, &NONE).unwrap(),
            dir.path().join("aula-1.mp4")
        );
    }

    #[test]
    fn saida_explicita_existente_sem_overwrite_falha() {
        let (dir, input) = setup();
        let final_ = dir.path().join("final.mp4");
        std::fs::write(&final_, b"x").unwrap();
        let err = resolve(&input, Some(&final_), CUT, false, &NONE).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("--overwrite"));
        assert_eq!(
            resolve(&input, Some(&final_), CUT, true, &NONE).unwrap(),
            final_
        );
    }

    #[test]
    fn saida_igual_a_entrada_e_recusada_mesmo_com_overwrite() {
        let (_dir, input) = setup();
        let err = resolve(&input, Some(&input), CUT, true, &NONE).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("entrada"));
    }

    #[cfg(unix)]
    #[test]
    fn saida_via_symlink_para_entrada_e_recusada() {
        let (dir, input) = setup();
        let link = dir.path().join("link.mp4");
        std::os::unix::fs::symlink(&input, &link).unwrap();
        let err = resolve(&input, Some(&link), CUT, true, &NONE).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn saida_relativa_com_dot_dot_para_entrada_e_recusada() {
        let (dir, input) = setup();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let tricky = sub.join("../aula.mp4");
        assert!(resolve(&input, Some(&tricky), CUT, true, &NONE).is_err());
    }

    #[test]
    fn saida_explicita_diretorio_usa_nome_padrao_dentro() {
        let (dir, input) = setup();
        let sub = dir.path().join("saidas");
        std::fs::create_dir(&sub).unwrap();
        assert_eq!(
            resolve(&input, Some(&sub), CUT, false, &NONE).unwrap(),
            sub.join("aula.cut.mp4")
        );
    }

    #[test]
    fn mesmo_arquivo_comprimido_duas_vezes() {
        let (dir, input) = setup();
        let compressed = DefaultName {
            suffix: Some("compressed"),
            ext: "mp4",
        };
        let mut reserved = Reservations::new();
        let first = resolve(&input, None, compressed, false, &reserved).unwrap();
        assert_eq!(first, dir.path().join("aula.compressed.mp4"));
        reserved.insert(&first);
        let second = resolve(&input, None, compressed, false, &reserved).unwrap();
        assert_eq!(second, dir.path().join("aula.compressed-1.mp4"));
        // liberada a reserva (job descartado), o nome volta a ficar livre
        reserved.remove(&first);
        assert_eq!(
            resolve(&input, None, compressed, false, &reserved).unwrap(),
            first
        );
    }

    #[test]
    fn caminho_explicito_repetido_e_recusado() {
        let (dir, input) = setup();
        let final_ = dir.path().join("final.mkv");
        let mut reserved = Reservations::new();
        reserved.insert(&final_);
        let err = resolve(&input, Some(&final_), CUT, true, &reserved).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("outro job da fila"), "{err}");
        // a mesma reserva por um caminho relativo equivalente também é detectada
        let rel = dir.path().join(".").join("final.mkv");
        assert!(resolve(&input, Some(&rel), CUT, true, &reserved).is_err());
    }
}
