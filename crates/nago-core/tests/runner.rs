use std::ffi::OsString;
use std::time::Duration;

use nago_core::runner::{JobEvent, Runner, TempOutput};
use nago_core::tools::Tools;
use tokio_util::sync::CancellationToken;

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

#[tokio::test]
async fn emite_progresso_crescente_e_termina() {
    let tools = Tools::locate().await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let runner = Runner::new(tools).with_events(tx);
    let dir = tempfile::tempdir().unwrap();
    let out = TempOutput::new(&dir.path().join("x.mp4")).unwrap();
    let mut args = os(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=1280x720:rate=30:duration=8",
        "-c:v",
        "libx264",
        "-preset",
        "medium",
        "-f",
        "mp4",
    ]);
    args.push(out.path().into());
    runner.run_ffmpeg("codificando", &args, 8.0).await.unwrap();
    drop(runner);

    let mut events = Vec::new();
    while let Some(e) = rx.recv().await {
        events.push(e);
    }
    assert!(matches!(&events[0], JobEvent::Phase { name } if name == "codificando"));
    let ratios: Vec<f64> = events
        .iter()
        .filter_map(|e| match e {
            JobEvent::Progress(p) => Some(p.ratio),
            _ => None,
        })
        .collect();
    assert!(ratios.len() >= 2, "poucos eventos: {ratios:?}");
    assert!(
        ratios.windows(2).all(|w| w[1] >= w[0]),
        "não crescente: {ratios:?}"
    );
    assert_eq!(*ratios.last().unwrap(), 1.0);
    let final_path = out.persist().unwrap();
    assert!(final_path.metadata().unwrap().len() > 0);
}

#[tokio::test]
async fn falha_do_ffmpeg_nao_deixa_arquivo_final_nem_temporario() {
    let tools = Tools::locate().await.unwrap();
    let runner = Runner::new(tools);
    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("final.mp4");
    {
        let out = TempOutput::new(&final_path).unwrap();
        let mut args = os(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=duration=2",
            "-c:v",
            "encoder_que_nao_existe",
        ]);
        args.push(out.path().into());
        let err = runner
            .run_ffmpeg("codificando", &args, 2.0)
            .await
            .unwrap_err();
        match &err {
            nago_core::Error::FfmpegFailed { log_tail, .. } => {
                assert!(
                    log_tail
                        .iter()
                        .any(|l| l.contains("encoder_que_nao_existe")),
                    "{log_tail:?}"
                );
            }
            other => panic!("erro inesperado: {other:?}"),
        }
        assert_eq!(err.exit_code(), 1);
    }
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "sobrou arquivo"
    );
}

#[tokio::test]
async fn cancelamento_encerra_o_ffmpeg_e_limpa() {
    let tools = Tools::locate().await.unwrap();
    let token = CancellationToken::new();
    let runner = Runner::new(tools).with_cancel(token.clone());
    let dir = tempfile::tempdir().unwrap();
    let final_path = dir.path().join("longo.mp4");
    let started = std::time::Instant::now();
    {
        let out = TempOutput::new(&final_path).unwrap();
        let mut args = os(&[
            "-re",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=duration=60",
            "-c:v",
            "libx264",
            "-f",
            "mp4",
        ]);
        args.push(out.path().into());
        let t = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(800)).await;
            t.cancel();
        });
        let err = runner
            .run_ffmpeg("codificando", &args, 60.0)
            .await
            .unwrap_err();
        assert!(matches!(err, nago_core::Error::Cancelled), "{err:?}");
        assert_eq!(err.exit_code(), 130);
    }
    assert!(started.elapsed() < Duration::from_secs(8));
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "sobrou arquivo"
    );
}
