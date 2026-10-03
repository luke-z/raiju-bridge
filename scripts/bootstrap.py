"""Fetch the pinned GPUI source and apply its build-only cross-compilation fix."""
import hashlib
import io
from pathlib import Path
import tarfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
GPUI_SHA256 = "979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707"


def download(url, sha256):
    request = urllib.request.Request(url, headers={"User-Agent": "raiju-bridge-build"})
    with urllib.request.urlopen(request, timeout=120) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != sha256:
        raise RuntimeError(f"Checksum mismatch: {url}")
    return data


def bootstrap():
    destination = ROOT / ".build" / "gpui"
    marker = destination / ".raiju-cross-v1"
    if marker.exists():
        return
    archive = download("https://static.crates.io/crates/gpui/gpui-0.2.2.crate", GPUI_SHA256)
    destination.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as source:
        for member in source.getmembers():
            relative = Path(member.name).relative_to("gpui-0.2.2")
            if member.isfile():
                path = destination / relative
                if not path.resolve().is_relative_to(destination.resolve()):
                    raise RuntimeError("Unsafe crate path")
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(source.extractfile(member).read())
    # Upstream selects the Windows build by TARGET but gates its implementation
    # on the host OS. The patch makes shader and manifest builds usable on Linux.
    build = destination / "build.rs"
    text = build.read_text(encoding="utf-8")
    changes = [
        ('            #[cfg(target_os = "windows")]\n            windows::build();', '            windows::build();'),
        ('#[cfg(target_os = "windows")]\nmod windows {', 'mod windows {'),
    ]
    for before, after in changes:
        if text.count(before) != 1:
            raise RuntimeError("GPUI build script changed; review the cross patch")
        text = text.replace(before, after)
    build.write_text(text, encoding="utf-8", newline="\n")
    manifest = destination / "Cargo.toml"
    text = manifest.read_text(encoding="utf-8")
    before = '[target.\'cfg(target_os = "windows")\'.build-dependencies.embed-resource]'
    if text.count(before) != 1:
        raise RuntimeError("GPUI manifest changed; review the cross patch")
    manifest.write_text(text.replace(before, "[build-dependencies.embed-resource]"), encoding="utf-8", newline="\n")
    marker.write_text(GPUI_SHA256, encoding="utf-8")


if __name__ == "__main__":
    bootstrap()
