# Calibração dos perfis de qualidade

Como foram escolhidos os valores de CRF/CQ da tabela em
`crates/nago-core/src/profiles.rs` (decisão D6 em `openspec/changes/archive/2026-11-19-add-core-cli/design.md`).

## Metas

| Perfil | Meta (VMAF médio) |
|---|---|
| `visually-lossless` | ≥ 95 |
| `high` (padrão) | ≥ 93 |
| `balanced` | ≥ 91 |
| `small` | ≥ 88 |

O modelo `vmaf_v0.6.1` satura em cerca de 97,5 mesmo comparando um vídeo com ele
mesmo em conteúdo de pouco movimento. Por isso as metas ficam abaixo desse teto.

## Corpus

| Clipe | Conteúdo | Formato | Duração |
|---|---|---|---|
| A | gravação de celular já comprimida | H.264 848x478, 30 fps | 47 s |
| B | gravação de celular já comprimida | H.264 1080x1920, 30 fps | 60 s |
| C | *Tears of Steel* (live action, CC-BY Blender Foundation), trecho a partir de 300 s | H.264 1280x534, 24 fps | 20 s |
| D | *Sintel* (animação, CC-BY Blender Foundation), trecho a partir de 420 s | H.264 1920x818, 24 fps | 26 s |

## Método

1. **Varredura:** cada encoder foi rodado com uma grade de CRF/CQ nos 4 clipes, com os
   mesmos parâmetros do `profiles.rs` (só o vídeo, sem áudio). Grades: SVT-AV1 14–38,
   x265 12–30, x264 12–27, NVENC AV1 18–42, NVENC HEVC/H.264 14–32.
2. **Medição:** VMAF `vmaf_v0.6.1` em 10-bit, com os frames pareados por índice
   (`settb=AVTB,setpts=N`), igual ao `--verify`.
3. **Escolha:** para cada perfil, o maior CRF/CQ cujo VMAF médio atinge a meta no
   **pior** clipe, interpolando linearmente entre os pontos da grade e arredondando
   para baixo.
4. **Verificação:** os valores escolhidos foram medidos de novo nos 4 clipes
   (tabela abaixo).

Em todos os encoders, o pior clipe foi uma gravação de celular (A). Fontes já
comprimidas têm artefatos que o encoder precisa reproduzir, o que custa bitrate.
Conteúdo limpo (C, D) fica bem acima da meta e reduz muito mais.

## Resultado da verificação

VMAF médio por clipe, com a variação de tamanho só do vídeo entre parênteses.

| Encoder | Perfil | CRF/CQ | A | B | C | D | Pior |
|---|---|---|---|---|---|---|---|
| libsvtav1 | visually-lossless | 15 | 95.2 (+5%) | 95.8 (+6%) | 98.7 (-2%) | 97.5 (-77%) | 95.16 ✓ |
| libsvtav1 | high | 24 | 93.3 (-39%) | 94.5 (-41%) | 97.7 (-51%) | 96.6 (-89%) | 93.32 ✓ |
| libsvtav1 | balanced | 30 | 91.2 (-60%) | 93.0 (-62%) | 96.4 (-70%) | 95.6 (-93%) | 91.18 ✓ |
| libsvtav1 | small | 35 | 88.5 (-72%) | 91.2 (-74%) | 94.5 (-79%) | 94.4 (-95%) | 88.53 ✓ |
| libx265 | visually-lossless | 17 | 95.2 (-5%) | 96.1 (+12%) | 98.9 (-13%) | 97.9 (-74%) | 95.21 ✓ |
| libx265 | high | 21 | 93.5 (-40%) | 95.0 (-32%) | 98.2 (-51%) | 97.3 (-86%) | 93.49 ✓ |
| libx265 | balanced | 24 | 91.4 (-59%) | 93.6 (-54%) | 97.2 (-68%) | 96.5 (-91%) | 91.37 ✓ |
| libx265 | small | 27 | 88.4 (-71%) | 91.5 (-69%) | 95.2 (-79%) | 95.2 (-94%) | 88.37 ✓ |
| libx264 | visually-lossless | 15 | 95.4 (+36%) | 96.3 (+79%) | 98.8 (+6%) | 97.9 (-32%) | 95.40 ✓ |
| libx264 | high | 19 | 93.5 (-7%) | 95.2 (+21%) | 97.8 (-37%) | 97.0 (-68%) | 93.48 ✓ |
| libx264 | balanced | 21 | 91.9 (-23%) | 94.2 (+0%) | 96.9 (-52%) | 96.3 (-76%) | 91.94 ✓ |
| libx264 | small | 24 | 88.7 (-44%) | 92.1 (-27%) | 94.6 (-67%) | 94.7 (-85%) | 88.70 ✓ |
| av1_nvenc | visually-lossless | 27 | 95.1 (+23%) | 95.9 (+52%) | 98.1 (-41%) | 97.8 (-64%) | 95.07 ✓ |
| av1_nvenc | high | 32 | 93.4 (-23%) | 94.7 (-10%) | 96.4 (-65%) | 97.2 (-81%) | 93.38 ✓ |
| av1_nvenc | balanced | 36 | 91.4 (-46%) | 93.4 (-41%) | 94.1 (-77%) | 96.3 (-88%) | 91.36 ✓ |
| av1_nvenc | small | 40 | 88.6 (-62%) | 91.3 (-61%) | 90.8 (-84%) | 94.9 (-92%) | 88.57 ✓ |
| hevc_nvenc | visually-lossless | 21 | 95.2 (+54%) | 96.1 (+94%) | 98.7 (-14%) | 97.9 (-48%) | 95.25 ✓ |
| hevc_nvenc | high | 26 | 93.1 (-10%) | 94.7 (+12%) | 96.8 (-58%) | 97.0 (-75%) | 93.05 ✓ |
| hevc_nvenc | balanced | 28 | 91.7 (-29%) | 93.8 (-13%) | 95.3 (-69%) | 96.4 (-81%) | 91.67 ✓ |
| hevc_nvenc | small | 31 | 88.9 (-50%) | 92.0 (-40%) | 92.3 (-79%) | 95.1 (-88%) | 88.94 ✓ |
| h264_nvenc | visually-lossless | 23 | 95.3 (+45%) | 96.0 (+83%) | 98.0 (-26%) | 97.4 (-52%) | 95.34 ✓ |
| h264_nvenc | high | 27 | 93.5 (+0%) | 94.6 (+27%) | 95.3 (-57%) | 96.3 (-71%) | 93.45 ✓ |
| h264_nvenc | balanced | 29 | 91.8 (-18%) | 93.4 (+6%) | 93.0 (-68%) | 95.3 (-77%) | 91.78 ✓ |
| h264_nvenc | small | 32 | 88.1 (-39%) | 90.7 (-22%) | 88.3 (-78%) | 93.2 (-84%) | 88.06 ✓ |

Todas as 24 combinações atingem a meta no pior clipe.

## Observações

- **`visually-lossless` em fontes já comprimidas:** o arquivo pode crescer. Por exemplo,
  SVT-AV1 +5%/+6% e NVENC HEVC +54%/+94% nos clipes A/B. Nesses casos a proteção de
  tamanho descarta a saída (código 3), a menos que `--force` seja usado.
- **GPU:** NVENC atinge as mesmas metas, mas com arquivos bem maiores que o software.
  No perfil `high`, NVENC AV1 reduz A/B em −23%/−10%, contra −39%/−41% do SVT-AV1.
- **QSV e VAAPI:** usam os mesmos valores do NVENC, sem calibração própria (não havia
  hardware disponível).
- **Máquina de referência:** Ryzen 7 5800X3D, RTX 4060 Ti, FFmpeg 9.0.2 (SVT-AV1 4.2.0).
  A calibração foi feita em 2026-11-19.
