//! Fixtures de mídia sintéticas geradas com `ffmpeg -f lavfi`.
//!
//! Cada fixture é gerada uma única vez em `target/nago-fixtures/` e reutilizada
//! por todos os testes (e processos de teste). A geração escreve num arquivo
//! temporário e renomeia ao final, então processos concorrentes não corrompem nada.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Fixtures disponíveis. Todas usam GOP fixo de 2 s (keyframe a cada 60 frames a 30 fps),
/// o que torna previsível o comportamento do corte rápido.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fixture {
    /// MP4, H.264 1920x1080 30 fps + AAC estéreo, 10 s.
    H264Aac1080p,
    /// MP4, H.264 640x360 30 fps + AAC estéreo, 10 s (para testes que recodificam).
    H264Aac360p,
    /// MKV, HEVC 10-bit HDR10 (PQ/BT.2020 com mastering display e MaxCLL) 640x360, 3 s.
    HevcHdr10,
    /// MOV, H.264 640x360 + áudio PCM s16le, 3 s.
    MovPcm,
    /// MKV, H.264 640x360 + 2 áudios AAC (por, eng) + 2 legendas SRT (por, eng), 6 s.
    MkvMulti,
    /// MKV, AV1 de bitrate muito baixo 640x360 + Opus, 5 s.
    Av1Low,
}

impl Fixture {
    pub const ALL: [Fixture; 6] = [
        Fixture::H264Aac1080p,
        Fixture::H264Aac360p,
        Fixture::HevcHdr10,
        Fixture::MovPcm,
        Fixture::MkvMulti,
        Fixture::Av1Low,
    ];

    fn file_name(self) -> &'static str {
        match self {
            Fixture::H264Aac1080p => "h264_aac_1080p.mp4",
            Fixture::H264Aac360p => "h264_aac_360p.mp4",
            Fixture::HevcHdr10 => "hevc_hdr10.mkv",
            Fixture::MovPcm => "h264_pcm.mov",
            Fixture::MkvMulti => "multi_tracks.mkv",
            Fixture::Av1Low => "av1_low.mkv",
        }
    }

    /// Argumentos do ffmpeg (sem o caminho de saída).
    fn args(self, dir: &Path) -> Vec<String> {
        const GOP: &str = "-g 60 -keyint_min 60 -sc_threshold 0";
        let video = |size: &str, secs: u32| {
            format!("-f lavfi -i testsrc2=size={size}:rate=30:duration={secs}")
        };
        let sine = |freq: u32, secs: u32| {
            format!("-f lavfi -i sine=frequency={freq}:sample_rate=48000:duration={secs}")
        };
        let s = match self {
            Fixture::H264Aac1080p => format!(
                "{} {} -map 0:v -map 1:a -c:v libx264 -preset ultrafast -crf 23 -pix_fmt yuv420p {GOP} \
                 -c:a aac -b:a 128k -ac 2 -f mp4",
                video("1920x1080", 10),
                sine(440, 10)
            ),
            Fixture::H264Aac360p => format!(
                "{} {} -map 0:v -map 1:a -c:v libx264 -preset veryfast -crf 20 -pix_fmt yuv420p {GOP} \
                 -c:a aac -b:a 128k -ac 2 -f mp4",
                video("640x360", 10),
                sine(440, 10)
            ),
            Fixture::HevcHdr10 => format!(
                "{} -map 0:v -vf setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc \
                 -c:v libx265 -preset ultrafast -pix_fmt yuv420p10le \
                 -x265-params log-level=error:keyint=60:min-keyint=60:scenecut=0:hdr10=1:repeat-headers=1:\
                 colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:\
                 master-display=G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,50):\
                 max-cll=1000,400 -f matroska",
                video("640x360", 3)
            ),
            Fixture::MovPcm => format!(
                "{} {} -map 0:v -map 1:a -c:v libx264 -preset veryfast -pix_fmt yuv420p {GOP} \
                 -c:a pcm_s16le -ac 2 -f mov",
                video("640x360", 3),
                sine(440, 3)
            ),
            Fixture::MkvMulti => {
                let srt_por = dir.join("legenda_por.srt");
                let srt_eng = dir.join("legenda_eng.srt");
                std::fs::write(&srt_por, SRT_POR).expect("escrever srt");
                std::fs::write(&srt_eng, SRT_ENG).expect("escrever srt");
                format!(
                    "{} {} {} -i {} -i {} -map 0:v -map 1:a -map 2:a -map 3:s -map 4:s \
                     -c:v libx264 -preset veryfast -pix_fmt yuv420p {GOP} -c:a aac -b:a 96k -ac 2 -c:s srt \
                     -metadata:s:a:0 language=por -metadata:s:a:1 language=eng \
                     -metadata:s:s:0 language=por -metadata:s:s:1 language=eng \
                     -disposition:a:0 default -disposition:a:1 0 -f matroska",
                    video("640x360", 6),
                    sine(440, 6),
                    sine(880, 6),
                    srt_por.display(),
                    srt_eng.display()
                )
            }
            Fixture::Av1Low => format!(
                "{} {} -map 0:v -map 1:a -c:v libsvtav1 -preset 12 -crf 63 -g 60 -pix_fmt yuv420p \
                 -svtav1-params enable-overlays=0 -b:v 0 -c:a libopus -b:a 24k -f matroska",
                video("640x360", 5),
                sine(440, 5)
            ),
        };
        s.split_whitespace().map(str::to_owned).collect()
    }
}

const SRT_POR: &str = "1\n00:00:00,500 --> 00:00:02,000\nOlá, mundo\n\n2\n00:00:03,000 --> 00:00:05,000\nSegunda legenda\n";
const SRT_ENG: &str = "1\n00:00:00,500 --> 00:00:02,000\nHello, world\n\n2\n00:00:03,000 --> 00:00:05,000\nSecond subtitle\n";

/// Diretório compartilhado das fixtures (`target/nago-fixtures`).
pub fn fixtures_dir() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"));
    let dir = target.join("nago-fixtures");
    std::fs::create_dir_all(&dir).expect("criar diretório de fixtures");
    dir
}

/// Caminho da fixture, gerando-a se ainda não existir.
pub fn fixture(f: Fixture) -> PathBuf {
    let dir = fixtures_dir();
    let path = dir.join(f.file_name());
    if path.exists() {
        return path;
    }
    let tmp = dir.join(format!(".{}.{}.tmp", f.file_name(), std::process::id()));
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(f.args(&dir))
        .arg(&tmp)
        .output()
        .expect("executar ffmpeg (ele está instalado?)");
    assert!(
        out.status.success(),
        "falha ao gerar a fixture {f:?}:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::rename(&tmp, &path).expect("renomear fixture");
    path
}

/// Copia uma fixture para `dir` (para testes que gravam ao lado da entrada).
pub fn fixture_in(f: Fixture, dir: &Path) -> PathBuf {
    let dest = dir.join(f.file_name());
    std::fs::copy(fixture(f), &dest).expect("copiar fixture");
    dest
}

/// Executa o ffprobe e devolve o JSON de format + streams.
pub fn ffprobe_json(path: &Path) -> serde_json::Value {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .expect("executar ffprobe");
    assert!(out.status.success(), "ffprobe falhou em {}", path.display());
    serde_json::from_slice(&out.stdout).expect("JSON do ffprobe")
}

/// Atalho: lista de `codec_name` dos streams.
pub fn stream_codecs(path: &Path) -> Vec<String> {
    ffprobe_json(path)["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["codec_name"].as_str().unwrap_or("").to_owned())
        .collect()
}

/// O ffmpeg instalado tem o filtro libvmaf? (Ubuntu, por exemplo, não traz.)
pub fn has_libvmaf() -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-filters"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(" libvmaf "))
        .unwrap_or(false)
}

/// Duração do container em segundos.
pub fn duration_secs(path: &Path) -> f64 {
    ffprobe_json(path)["format"]["duration"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gera_e_le_todas_as_fixtures() {
        for f in Fixture::ALL {
            let path = fixture(f);
            let json = ffprobe_json(&path);
            let streams = json["streams"].as_array().unwrap();
            let expected = match f {
                Fixture::HevcHdr10 => 1,
                Fixture::MkvMulti => 5,
                _ => 2,
            };
            assert_eq!(streams.len(), expected, "{f:?}");
        }
        let hdr = ffprobe_json(&fixture(Fixture::HevcHdr10));
        assert_eq!(hdr["streams"][0]["color_transfer"], "smpte2084");
        assert_eq!(hdr["streams"][0]["pix_fmt"], "yuv420p10le");
        let pcm = stream_codecs(&fixture(Fixture::MovPcm));
        assert_eq!(pcm, ["h264", "pcm_s16le"]);
        let av1 = stream_codecs(&fixture(Fixture::Av1Low));
        assert_eq!(av1, ["av1", "opus"]);
    }
}
