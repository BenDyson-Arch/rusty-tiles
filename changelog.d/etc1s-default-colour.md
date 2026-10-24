### Changed

- Mesh colour textures (leaf and coarse base colour/emissive pages) now default
  to KTX2 ETC1S: quality 50 in `fast`, 90 in `balanced`, effort 0 in both.
  `quality` keeps UASTC and `lossless` keeps PNG. ETC1S accepts more RGB error
  than the previous
  UASTC gates (24/20 dB for large/small): per-channel PSNR floors are now
  20/16/13 dB for large/middle/tiny supported levels, with wider local-error
  and decoded sampler caps. Alpha,
  MASK, data, strict authored correspondence and escalation remain active.
  Tiny levels additionally require combined RGB PSNR of 16 dB, rejecting
  correlated colour damage while admitting one-channel ETC1S palette tails.
  On tex3 the matched two-worker sweep costs 47.1/56.5 CPU-s for fast/balanced
  raw encoding, versus 16.1/18.2 CPU-s for the previous UASTC pack 0 codec.
  Effort 0 reduces the initial effort 3 cost substantially, but ETC1S still
  costs about 3× that previous codec's encode CPU in exchange for fewer bytes.
  Smooth wrong-mip content can pass these PSNR gates; gross wrong-mip,
  black-page, inverted-channel and light-grey wrong-transfer controls fail.
