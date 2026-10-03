$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
python scripts/bootstrap.py
if ($LASTEXITCODE -ne 0) { throw 'Dependency preparation failed.' }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'Install the Rust stable MSVC toolchain first.' }
$sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$fxc = Get-ChildItem -LiteralPath $sdkRoot -Directory | Sort-Object Name -Descending | ForEach-Object { Join-Path $_.FullName 'x64\fxc.exe' } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $fxc) { throw 'Windows SDK x64 fxc.exe is required by GPUI. Install the C++ desktop build tools and Windows SDK.' }
$env:PATH = (Split-Path -Parent $fxc) + ';' + $env:PATH
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed.' }
cargo clippy --locked --release --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'Lint failed.' }
cargo test --locked --release --lib
if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
cargo build --locked --release --bins
if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
Write-Host 'Binaries are in target\release. Run python scripts/build_backend.py to build the compatible DLL (see docs/development.md).'
