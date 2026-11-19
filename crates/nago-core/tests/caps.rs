use nago_core::caps;
use nago_core::encoders::Encoder;
use nago_core::tools::Tools;

#[tokio::test]
async fn encoder_inexistente_e_marcado_indisponivel_com_motivo() {
    // Um ffmpeg "falso" que lista um encoder NVENC mas falha ao usá-lo.
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("ffmpeg");
    std::fs::write(
        &fake,
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *-encoders*) printf ' ------\\n V....D libsvtav1  SVT\\n V....D av1_nvenc  NVENC\\n';;\n\
         *-filters*) printf ' .. libvmaf  VV->V  x\\n';;\n\
         *-version*) echo 'ffmpeg version 9.0.2';;\n\
         *) echo '[av1_nvenc @ 0x1] Cannot load libcuda.so.1' >&2; exit 1;;\n\
         esac\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let tools = Tools {
        ffmpeg: fake.clone(),
        ffprobe: fake,
        version: nago_core::tools::Version::parse("ffmpeg version 9.0.2").unwrap(),
    };
    let caps = caps::detect(&tools).await.unwrap();
    assert!(caps.vmaf);
    let nv = Encoder::by_name("av1_nvenc").unwrap();
    match caps.status(&nv) {
        caps::EncoderStatus::Unavailable(reason) => assert!(reason.contains("libcuda"), "{reason}"),
        other => panic!("esperava indisponível, veio {other:?}"),
    }
    assert!(caps.is_usable(&Encoder::by_name("libsvtav1").unwrap()));
}

/// Roda na máquina de referência (RTX 4060 Ti): `cargo test -- --ignored`.
#[tokio::test]
#[ignore = "requer GPU NVIDIA com NVENC AV1"]
async fn av1_nvenc_funcional_na_gpu_local() {
    let tools = Tools::locate().await.unwrap();
    let caps = caps::detect(&tools).await.unwrap();
    for name in ["av1_nvenc", "hevc_nvenc", "h264_nvenc"] {
        assert!(
            caps.is_usable(&Encoder::by_name(name).unwrap()),
            "{name}: {:?}",
            caps.status(&Encoder::by_name(name).unwrap())
        );
    }
}
