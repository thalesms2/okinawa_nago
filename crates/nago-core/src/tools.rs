//! Localização de `ffmpeg`/`ffprobe` e verificação de versão
//! (spec `encoder-selection` › Verificação do FFmpeg).

use std::fmt;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Versão mínima suportada.
pub const MIN_VERSION: Version = Version::Release {
    major: 6,
    minor: 0,
    patch: 0,
};

const INSTALL_HINT: &str = "instale o ffmpeg (Arch: `sudo pacman -S ffmpeg`, Debian/Ubuntu: \
`sudo apt install ffmpeg`, Fedora: `sudo dnf install ffmpeg`, macOS: `brew install ffmpeg`) \
ou aponte NAGO_FFMPEG/NAGO_FFPROBE para os binários";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(untagged)]
pub enum Version {
    Release {
        major: u32,
        minor: u32,
        patch: u32,
    },
    /// Build do git (`N-112345-gabcdef`): sem número de versão, assumida recente.
    Git(String),
}

impl Version {
    /// Extrai a versão da primeira linha de `ffmpeg -version`.
    pub fn parse(version_output: &str) -> Option<Version> {
        let first = version_output.lines().next()?;
        let token = first
            .split_whitespace()
            .skip_while(|w| *w != "version")
            .nth(1)?;
        if token.starts_with("N-") {
            return Some(Version::Git(token.to_owned()));
        }
        let token = token.trim_start_matches('n');
        let numeric: String = token
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let mut parts = numeric.split('.').filter(|p| !p.is_empty());
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        Some(Version::Release {
            major,
            minor,
            patch,
        })
    }

    pub fn is_supported(&self) -> bool {
        match (self, &MIN_VERSION) {
            (Version::Git(_), _) => true,
            (
                Version::Release {
                    major,
                    minor,
                    patch,
                },
                Version::Release {
                    major: mj,
                    minor: mn,
                    patch: pt,
                },
            ) => (major, minor, patch) >= (mj, mn, pt),
            _ => false,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Version::Release {
                major,
                minor,
                patch,
            } => write!(f, "{major}.{minor}.{patch}"),
            Version::Git(s) => write!(f, "{s} (build do git)"),
        }
    }
}

/// Binários do FFmpeg localizados e validados.
#[derive(Debug, Clone)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub version: Version,
}

impl Tools {
    /// Localiza os binários (`NAGO_FFMPEG`/`NAGO_FFPROBE` > PATH) e valida a versão.
    pub async fn locate() -> Result<Tools> {
        let ffmpeg = find("ffmpeg", "NAGO_FFMPEG")?;
        let ffprobe = find("ffprobe", "NAGO_FFPROBE")?;
        let version = version_of(&ffmpeg).await?;
        if !version.is_supported() {
            return Err(Error::Ffmpeg(format!(
                "ffmpeg {version} encontrado em {}, mas o nago exige a versão {MIN_VERSION} ou superior; {INSTALL_HINT}",
                ffmpeg.display()
            )));
        }
        Ok(Tools {
            ffmpeg,
            ffprobe,
            version,
        })
    }
}

fn find(name: &str, env_var: &str) -> Result<PathBuf> {
    if let Some(custom) = std::env::var_os(env_var) {
        let path = PathBuf::from(custom);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(Error::Ffmpeg(format!(
                "{env_var} aponta para {}, que não existe; {INSTALL_HINT}",
                path.display()
            )))
        };
    }
    which::which(name).map_err(|_| {
        Error::Ffmpeg(format!(
            "o nago precisa do `{name}`, que não foi encontrado no PATH; {INSTALL_HINT}"
        ))
    })
}

async fn version_of(ffmpeg: &Path) -> Result<Version> {
    let out = tokio::process::Command::new(ffmpeg)
        .arg("-version")
        .output()
        .await
        .map_err(|e| Error::Ffmpeg(format!("falha ao executar {}: {e}", ffmpeg.display())))?;
    let text = String::from_utf8_lossy(&out.stdout);
    Version::parse(&text).ok_or_else(|| {
        Error::Ffmpeg(format!(
            "não foi possível identificar a versão de {}",
            ffmpeg.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(major: u32, minor: u32, patch: u32) -> Version {
        Version::Release {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn parse_de_versoes() {
        let cases = [
            (
                "ffmpeg version n9.0.2 Copyright (c) 2000-2026",
                Some(rel(9, 0, 2)),
            ),
            (
                "ffmpeg version 6.1.1-3ubuntu5 Copyright (c) 2000-2023",
                Some(rel(6, 1, 1)),
            ),
            ("ffmpeg version 5.1 Copyright", Some(rel(5, 1, 0))),
            (
                "ffmpeg version 7.0.2-static https://johnvansickle.com",
                Some(rel(7, 0, 2)),
            ),
            (
                "ffmpeg version N-112345-g1234abcd Copyright",
                Some(Version::Git("N-112345-g1234abcd".into())),
            ),
            ("lixo", None),
            ("", None),
        ];
        for (input, expected) in cases {
            assert_eq!(Version::parse(input), expected, "{input}");
        }
    }

    #[test]
    fn versao_minima() {
        assert!(rel(9, 0, 2).is_supported());
        assert!(rel(6, 0, 0).is_supported());
        assert!(rel(6, 1, 1).is_supported());
        assert!(!rel(5, 1, 0).is_supported());
        assert!(!rel(4, 4, 9).is_supported());
        assert!(Version::Git("N-1".into()).is_supported());
    }
}
