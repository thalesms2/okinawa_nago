//! Parse e formatação de instantes no formato `SS[.ms]`, `MM:SS[.ms]` e `HH:MM:SS[.ms]`.

use crate::{Error, Result};

/// Converte um instante textual em segundos.
///
/// Aceita `90`, `90.5`, `1:30`, `1:30.5`, `01:02:03.250`. Componentes não iniciais
/// (minutos/segundos depois de `:`) precisam ser menores que 60.
pub fn parse(input: &str) -> Result<f64> {
    let s = input.trim();
    let invalid = |why: &str| {
        Error::usage(format!(
            "instante inválido `{input}`: {why} (use SS, MM:SS ou HH:MM:SS, com milissegundos opcionais, ex.: 1:30.5)"
        ))
    };
    if s.is_empty() {
        return Err(invalid("vazio"));
    }
    if s.starts_with('-') {
        return Err(invalid("não pode ser negativo"));
    }
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() > 3 {
        return Err(invalid("componentes demais"));
    }
    let (last, init) = parts.split_last().expect("split nunca é vazio");

    let seconds = parse_seconds(last).ok_or_else(|| invalid("segundos inválidos"))?;
    let mut total = seconds;
    if !init.is_empty() && seconds >= 60.0 {
        return Err(invalid("segundos devem ser menores que 60"));
    }

    let mut multiplier = 60.0;
    for (i, part) in init.iter().rev().enumerate() {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return Err(invalid("componente não numérico"));
        }
        let value: f64 = part.parse().map_err(|_| invalid("número inválido"))?;
        let is_leading = i == init.len() - 1;
        if !is_leading && value >= 60.0 {
            return Err(invalid("minutos devem ser menores que 60"));
        }
        total += value * multiplier;
        multiplier *= 60.0;
    }
    Ok(total)
}

fn parse_seconds(s: &str) -> Option<f64> {
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    if int.is_empty() || !int.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if let Some(f) = frac
        && (f.is_empty() || !f.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Formata segundos como `HH:MM:SS.mmm`.
pub fn format(secs: f64) -> String {
    let total_ms = (secs.max(0.0) * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    format!(
        "{:02}:{:02}:{:02}.{ms:03}",
        total_s / 3600,
        (total_s / 60) % 60,
        total_s % 60
    )
}

/// Formata segundos para a linha de comando do ffmpeg (precisão de microssegundos).
pub fn ffmpeg_arg(secs: f64) -> String {
    format!("{:.6}", secs.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instantes_validos() {
        let cases = [
            ("0", 0.0),
            ("90", 90.0),
            ("90.5", 90.5),
            ("1:30", 90.0),
            ("1:30.5", 90.5),
            ("2:45.5", 165.5),
            ("01:02:03.250", 3723.25),
            ("0:00:00.001", 0.001),
            ("100:00", 6000.0),
            ("  12  ", 12.0),
        ];
        for (input, expected) in cases {
            let got = parse(input).unwrap_or_else(|e| panic!("{input}: {e}"));
            assert!(
                (got - expected).abs() < 1e-9,
                "{input}: {got} != {expected}"
            );
        }
    }

    #[test]
    fn instantes_invalidos() {
        for input in [
            "", "-5", "-0:10", "abc", "1:2:3:4", "1:60", "1:60:00", "1::30", ":30", "1:", "1.2.3",
            "1:3a", "10.", ".5", "1e3", "inf", "NaN",
        ] {
            let err = parse(input).expect_err(input);
            assert!(matches!(err, Error::Usage(_)), "{input}");
            assert_eq!(err.exit_code(), 2);
        }
    }

    #[test]
    fn formatacao() {
        assert_eq!(format(0.0), "00:00:00.000");
        assert_eq!(format(88.4), "00:01:28.400");
        assert_eq!(format(3723.25), "01:02:03.250");
        assert_eq!(format(165.5), "00:02:45.500");
        assert_eq!(ffmpeg_arg(4.0005), "4.000500");
    }
}
