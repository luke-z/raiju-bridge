param(
    [string]$KitVersion = '10.0.26100.0',
    [ValidateRange(1,100)][int]$CertificateYears = 100
)
$ErrorActionPreference = 'Stop'
$prototypeRoot = $PSScriptRoot
$kitRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vsRoot = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (!$vsRoot) { throw 'Visual Studio C++ Build Tools are required to build the prototype.' }
$msbuild = Join-Path $vsRoot 'MSBuild\Current\Bin\MSBuild.exe'
$signtool = Join-Path $kitRoot "bin\$KitVersion\x64\signtool.exe"
$infverif = Join-Path $kitRoot "Tools\$KitVersion\x64\infverif.exe"
$inf2cat = Join-Path $kitRoot "bin\$KitVersion\x86\Inf2Cat.exe"
$package = Join-Path $prototypeRoot 'package'
New-Item -ItemType Directory -Path $package -Force | Out-Null
if (Get-Process -Name raiju-tray,raiju-vhf-probe -ErrorAction SilentlyContinue | Where-Object {
    $_.Path -in @((Join-Path $package 'raiju-tray.exe'),(Join-Path $package 'raiju-vhf-probe.exe'))
}) { throw 'Quit the tray app and diagnostic forwarder before rebuilding this package.' }

& $msbuild (Join-Path $prototypeRoot 'driver\RaijuVhf.vcxproj') /p:Configuration=Release /p:Platform=x64 /p:SpectreMitigation=false /p:RunCodeAnalysis=true /v:minimal /nologo
if ($LASTEXITCODE) { throw "Driver build failed: $LASTEXITCODE" }
# Spectre libraries are a build-only prerequisite absent on this PC; the private
# prototype uses ordinary MSVC static libraries. No OS mitigation is changed.
& cargo test --locked --manifest-path (Join-Path $prototypeRoot 'client\Cargo.toml') --release
if ($LASTEXITCODE) { throw 'Client tests failed' }
$previousRustFlags = $env:RUSTFLAGS
try {
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    & cargo build --locked --manifest-path (Join-Path $prototypeRoot 'client\Cargo.toml') --release
    if ($LASTEXITCODE) { throw 'Client build failed' }
} finally { $env:RUSTFLAGS = $previousRustFlags }

Copy-Item -LiteralPath (Join-Path $prototypeRoot 'build\driver\RaijuVhf.dll') -Destination $package -Force
Copy-Item -LiteralPath (Join-Path $prototypeRoot 'build\driver\RaijuVhf.pdb') -Destination $package -Force
Copy-Item -LiteralPath (Join-Path $prototypeRoot 'driver\RaijuVhf.inf') -Destination $package -Force
Copy-Item -LiteralPath (Join-Path $prototypeRoot 'client\target\release\raiju-vhf-probe.exe') -Destination $package -Force
Copy-Item -LiteralPath (Join-Path $prototypeRoot 'client\target\release\raiju-tray.exe') -Destination $package -Force

$certificate = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert | Where-Object {
    $_.Subject -eq 'CN=Raiju VHF Prototype Local Test' -and $_.HasPrivateKey -and
    $_.NotAfter -gt (Get-Date).AddDays(2) -and
    $_.NotAfter -ge $_.NotBefore.AddYears($CertificateYears).AddDays(-1) -and
    $_.NotAfter -le $_.NotBefore.AddYears($CertificateYears).AddDays(1)
} | Sort-Object NotAfter -Descending | Select-Object -First 1
if (!$certificate) {
    $certificate = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=Raiju VHF Prototype Local Test' -CertStoreLocation Cert:\CurrentUser\My -KeyAlgorithm RSA -KeyLength 2048 -HashAlgorithm SHA256 -KeyExportPolicy NonExportable -NotAfter (Get-Date).AddYears($CertificateYears)
}
Export-Certificate -Cert $certificate -FilePath (Join-Path $package 'RaijuVhf.cer') -Force | Out-Null
foreach ($name in @('RaijuVhf.dll','raiju-vhf-probe.exe','raiju-tray.exe')) {
    & $signtool sign /s My /sha1 $certificate.Thumbprint /fd SHA256 (Join-Path $package $name)
    if ($LASTEXITCODE) { throw "Local signing failed: $name" }
}
& $infverif /w (Join-Path $package 'RaijuVhf.inf')
if ($LASTEXITCODE) { throw 'INF validation failed' }
& $inf2cat "/driver:$package" /os:10_NI_X64,10_GE_X64 /uselocaltime
if ($LASTEXITCODE) { throw 'Catalog generation failed' }
& $signtool sign /s My /sha1 $certificate.Thumbprint /fd SHA256 (Join-Path $package 'RaijuVhf.cat')
if ($LASTEXITCODE) { throw 'Local catalog signing failed' }

$hashes = [ordered]@{}
foreach ($name in @('RaijuVhf.dll','RaijuVhf.inf','RaijuVhf.cat','RaijuVhf.cer','raiju-vhf-probe.exe','raiju-tray.exe')) {
    $hashes[$name] = (Get-FileHash -LiteralPath (Join-Path $package $name) -Algorithm SHA256).Hash
}
$versionMatch = [regex]::Match((Get-Content -LiteralPath (Join-Path $package 'RaijuVhf.inf') -Raw),'(?m)^DriverVer=[^,]+,([0-9.]+)')
if (!$versionMatch.Success) { throw 'Missing driver version' }
[ordered]@{ certificateSubject=$certificate.Subject; certificateThumbprint=$certificate.Thumbprint; certificateExpires=$certificate.NotAfter.ToUniversalTime().ToString('o'); certificateValidityYears=$CertificateYears; driverVersion=$versionMatch.Groups[1].Value; architecture='x64'; mode='identity-only and DualSense compatibility profiles'; files=$hashes } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $package 'manifest.json') -Encoding utf8
Write-Output "Prototype built and locally signed: $package"
Write-Output 'Building does not install a driver or trust the local signing certificate.'
