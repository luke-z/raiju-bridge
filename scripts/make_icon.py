"""Regenerate the shared executable, title-bar and tray icon (stdlib only)."""
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parent.parent


def color(x, y):
    bridge = (6 <= x < 11 or 21 <= x < 26) and 7 <= y < 26
    bridge |= 6 <= x < 26 and 7 <= y < 12
    return (217, 242, 126, 255) if bridge else (17, 25, 27, 255)


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def png(size):
    pixels = bytearray()
    for y in range(size):
        pixels.append(0)  # PNG's unfiltered scanline.
        for x in range(size):
            samples = [color((x + dx / 4 + 0.125) * 32 / size,
                             (y + dy / 4 + 0.125) * 32 / size)
                       for dy in range(4) for dx in range(4)]
            pixels.extend(sum(p[c] for p in samples) // 16 for c in range(4))
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(pixels, 9)) + chunk(b"IEND", b"")


def main():
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    images = [png(size) for size in sizes]
    directory = bytearray(struct.pack("<HHH", 0, 1, len(sizes)))
    offset = 6 + 16 * len(sizes)
    for size, data in zip(sizes, images):
        directory.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
        offset += len(data)
    (ROOT / "assets/app.ico").write_bytes(directory + b"".join(images))


if __name__ == "__main__":
    main()
