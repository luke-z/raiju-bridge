"""Extract Microsoft's pinned shader compiler for Wine; never ship it in the app."""
import io
from pathlib import Path
import zipfile
from bootstrap import ROOT, download

SDK_VERSION = "10.0.26100.3916"
SDK_SHA256 = "21dfb5db3397425ee428e0fd54cedc51af52ddd10c82433a7cdb483ecbb284ad"


def main():
    target = ROOT / ".build" / "fxc"
    target.mkdir(parents=True, exist_ok=True)
    data = download(
        f"https://api.nuget.org/v3-flatcontainer/microsoft.windows.sdk.cpp/{SDK_VERSION}/microsoft.windows.sdk.cpp.{SDK_VERSION}.nupkg",
        SDK_SHA256,
    )
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        for name in ["fxc.exe", "d3dcompiler_47.dll"]:
            (target / name).write_bytes(archive.read(f"c/bin/10.0.26100.0/x64/{name}"))


if __name__ == "__main__":
    main()
