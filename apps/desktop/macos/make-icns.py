"""Make Mothr.icns from icon.svg (run after changing the icon; the result is committed).

    python3 -I apps/desktop/macos/make-icns.py

Needs rsvg-convert (librsvg). An .icns file is a list of PNGs, one per size, so no macOS tools
are needed.
"""
import os, struct, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
# (type, pixels): 16..512 pt at 1x and 2x.
SIZES = [("icp4", 16), ("icp5", 32), ("ic11", 32), ("icp6", 64), ("ic12", 64), ("ic07", 128),
         ("ic13", 256), ("ic08", 256), ("ic14", 512), ("ic09", 512), ("ic10", 1024)]

def main():
    chunks = b""
    with tempfile.TemporaryDirectory() as tmp:
        for kind, px in SIZES:
            png = os.path.join(tmp, f"{px}.png")
            subprocess.run(["rsvg-convert", "-w", str(px), "-h", str(px), "-o", png, os.path.join(HERE, "icon.svg")], check=True)
            data = open(png, "rb").read()
            chunks += kind.encode() + struct.pack(">I", len(data) + 8) + data
    out = os.path.join(HERE, "Mothr.icns")
    with open(out, "wb") as f:
        f.write(b"icns" + struct.pack(">I", len(chunks) + 8) + chunks)
    print(f"wrote {out} ({os.path.getsize(out)} bytes)", file=sys.stderr)

main()
