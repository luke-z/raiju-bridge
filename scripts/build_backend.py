"""Build the pinned VIIPER library with the native DualSense USB report size."""
import hashlib
import io
import os
from pathlib import Path
import subprocess
import sys
import zipfile
from bootstrap import ROOT, download

BACKEND = "3111299d67bacbaa7f6a31d56ea9ae06678f5865"
SOURCE_SHA256 = "fc288cfc05f7611cae2d3063e8d8d9b6b98e1699c755a4766bc725d49228f8d4"
GO_VERSION = "1.27.1"


def prepare():
    cache = ROOT / ".build" / f"VIIPER-{BACKEND}.zip"
    cache.parent.mkdir(parents=True, exist_ok=True)
    if not cache.exists():
        cache.write_bytes(download(f"https://codeload.github.com/Alia5/VIIPER/zip/{BACKEND}", SOURCE_SHA256))
    archive = cache.read_bytes()
    if hashlib.sha256(archive).hexdigest() != SOURCE_SHA256:
        raise RuntimeError("VIIPER source checksum mismatch")
    destination = ROOT / ".build" / f"viiper-{BACKEND}-output48"
    with zipfile.ZipFile(io.BytesIO(archive)) as bundle:
        for item in bundle.infolist():
            if item.is_dir():
                continue
            relative = Path(item.filename).relative_to(f"VIIPER-{BACKEND}")
            path = destination / relative
            if not path.resolve().is_relative_to(destination.resolve()):
                raise RuntimeError("Unsafe VIIPER source path")
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(bundle.read(item))

    descriptor = destination / "device/dualsense/descriptor.go"
    text = descriptor.read_text(encoding="utf-8")
    before = "hid.Usage{Usage: 0x23},\n\t\t\t\t\t\t\thid.ReportCount{Count: 63},"
    if text.count(before) != 1:
        raise RuntimeError("VIIPER descriptor changed; review the DualSense compatibility patch")
    # Sony USB report 0x02 declares 47 payload bytes plus the report ID.
    # Upstream declares 63 + 1 instead, causing native games to reject the pad.
    # Reference: github.com/nondebug/dualsense/blob/main/report-descriptor-usb.txt
    descriptor.write_text(text.replace(before, before.replace("Count: 63", "Count: 47")), encoding="utf-8", newline="\n")
    return destination, archive


def build(binary_dir):
    source, archive = prepare()
    binary_dir = Path(binary_dir).resolve()
    binary_dir.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, GOOS="windows", GOARCH="amd64", CGO_ENABLED="1", GOTOOLCHAIN=f"go{GO_VERSION}")
    if os.name != "nt":
        env.setdefault("CC", "x86_64-w64-mingw32-gcc")
    subprocess.run([
        "go", "build", "-trimpath", "-buildmode=c-shared", "-ldflags=-s -w",
        "-o", str(binary_dir / "libVIIPER.dll"), "./lib/viiper",
    ], cwd=source, env=env, check=True)
    return archive


if __name__ == "__main__":
    build(sys.argv[1] if len(sys.argv) > 1 else ROOT / "target/release")
