### Changed

- `mesh` leaf UASTC textures are encoded with Basis Universal's fastest ETC1
  hints (`cPackUASTCETC1FastestHints | cPackUASTCETC1DisableFlipAndIndividual`):
  leaf texture encode CPU drops 35% on tex3 and 56% on Walem 2048. Decodes to
  RGBA, ASTC, BC7 and BC1/3 are unchanged; the ETC1 transcode, used only by GPUs
  without ASTC, BC7 or S3TC, loses about 0.6–0.7 dB. Leaf KTX2 bytes change
  (archives 0.3–0.7% smaller); coarse tiles are byte-identical.
