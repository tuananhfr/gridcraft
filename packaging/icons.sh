#!/usr/bin/env bash
# Derive desktop icons from the contributor-original ERPCons raster artwork.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python3 - "$ROOT/assets/app-icon" <<'PYTHON'
from pathlib import Path
from PIL import Image
import sys
root = Path(sys.argv[1])
source = Image.open(root / "gridcraft-1024.png").convert("RGBA")
for size in (16, 24, 32, 48, 64, 128, 256, 512):
    dest = root / "hicolor" / f"{size}x{size}" / "apps" / "vn.erpcons.bangtinh.png"
    dest.parent.mkdir(parents=True, exist_ok=True)
    source.resize((size, size), Image.Resampling.LANCZOS).save(dest)
source.save(root / "ERPCons-BangTinh.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
source.save(root / "gridcraft.icns")
PYTHON
