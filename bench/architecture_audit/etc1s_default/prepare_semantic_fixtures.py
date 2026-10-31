"""Pad exported codec base fixtures into valid semantic delivery pages.
Usage: python3 prepare_semantic_fixtures.py RAW_BASES_DIR OUTPUT_DIR
Literal interior pixels/support are retained; edge-extension padding has zero
support. This independently implements the page's POT/minimum-four domain.
"""
import sys
from pathlib import Path
source, dest = map(Path,sys.argv[1:]); dest.mkdir(parents=True,exist_ok=True)
for path in sorted(source.glob("*.rgba")):
    dims, name = path.stem.split("-",1); w,h = map(int,dims.split("x"))
    rgba = path.read_bytes(); support = path.with_suffix(".support").read_bytes()
    assert len(rgba)==w*h*4 and len(support)==w*h
    pw,ph = max(4,1<<(w-1).bit_length()),max(4,1<<(h-1).bit_length())
    image,mask = bytearray(pw*ph*4),bytearray(pw*ph)
    for y in range(ph):
        for x in range(pw):
            a=(min(y,h-1)*w+min(x,w-1))*4; b=(y*pw+x)*4
            image[b:b+4]=rgba[a:a+4]
            if x<w and y<h: mask[y*pw+x]=support[y*w+x]
    stem=dest/f"{pw}x{ph}-{name}"
    stem.with_suffix(".rgba").write_bytes(image)
    stem.with_suffix(".support").write_bytes(mask)
