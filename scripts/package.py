"""Build a portable Windows ZIP with pinned runtime and corresponding sources."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import zipfile
from bootstrap import ROOT, GPUI_SHA256, download
from build_backend import build as build_backend


def dependency_sources(bundle):
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--format-version", "1",
        "--filter-platform", "x86_64-pc-windows-msvc",
    ], cwd=ROOT))
    active = {node["id"] for node in metadata["resolve"]["nodes"]}
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    notices = []
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["id"] not in active or package["name"] == "raiju-bridge":
            continue
        name, version = package["name"], package["version"]
        notices.append(f"{name} {version} — {package['license']}\n{package.get('repository') or ''}\n")
        source = Path(package["manifest_path"]).parent
        licenses = [p for p in source.iterdir() if p.is_file() and p.name.upper().startswith(("LICENSE", "COPYING", "NOTICE", "COPYRIGHT"))]
        if package.get("license_file"):
            licenses.append(source / package["license_file"])
        for file in sorted(set(licenses)):
            notices.append(file.read_text(encoding="utf-8", errors="replace"))
        filename = f"{name}-{version}.crate"
        if name == "gpui":
            data = download(f"https://static.crates.io/crates/gpui/{filename}", GPUI_SHA256)
        else:
            matches = list((cargo_home / "registry/cache").glob(f"*/{filename}"))
            if len(matches) != 1:
                raise RuntimeError(f"Missing or ambiguous dependency archive: {filename}")
            data = matches[0].read_bytes()
        bundle.writestr(f"source/crates/{filename}", data)
    bundle.writestr("licenses/RUST-DEPENDENCIES.txt", "\n\n".join(notices))


def main():
    binary_dir = Path(sys.argv[1]).resolve()
    for name in ["raiju-bridge.exe", "raiju-bridge-cli.exe"]:
        if not (binary_dir / name).is_file():
            raise RuntimeError(f"Build first: {binary_dir / name}")
    # Always rebuild from the verified source and patch; never package an older
    # upstream DLL that would silently restore the incompatible descriptor.
    backend_source = build_backend(binary_dir)
    destination = ROOT / "dist"
    destination.mkdir(exist_ok=True)
    output = destination / "raiju-bridge-windows-x64.zip"
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as bundle:
        for name in ["raiju-bridge.exe", "raiju-bridge-cli.exe", "libVIIPER.dll"]:
            bundle.write(binary_dir / name, name)
        for name in ["README.md", "LICENSE", "THIRD_PARTY.md"]:
            bundle.write(ROOT / name, name)
        for name in ["compact.jpg", "diagnostics.jpg"]:
            bundle.write(ROOT / "docs/images" / name, f"docs/images/{name}")
        bundle.writestr("source/VIIPER-v0.8.2.zip", backend_source)
        # Use tracked files only: no build caches, captures, machine paths or logs.
        files = subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode().split("\0")
        if not any(name == "Cargo.toml" for name in files):
            raise RuntimeError("Package from a checked-out or staged source tree")
        for name in filter(None, files):
            bundle.write(ROOT / name, f"source/raiju-bridge/{name}")
        dependency_sources(bundle)
    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    (destination / "SHA256SUMS.txt").write_text(f"{digest}  {output.name}\n", encoding="utf-8", newline="\n")
    print(output)


if __name__ == "__main__":
    main()
