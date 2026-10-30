"""Packs assets/icon-{16,24,32,48,64,256}.png into packaging/windows/leaf9000.ico
(PNG entries, which Windows Vista and later read). Run after
scripts/make_icon.swift."""
import struct
from pathlib import Path

root = Path(__file__).resolve().parent.parent
assets = root / "assets"
sizes = [16, 24, 32, 48, 64, 256]
images = [(assets / f"icon-{s}.png").read_bytes() for s in sizes]
out = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
for s, data in zip(sizes, images):
    side = 0 if s == 256 else s  # 0 means 256
    out += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
(root / "packaging/windows/leaf9000.ico").write_bytes(out + b"".join(images))
for s in (16, 24, 32, 48, 64):
    (assets / f"icon-{s}.png").unlink()
print("wrote", root / "packaging/windows/leaf9000.ico")
