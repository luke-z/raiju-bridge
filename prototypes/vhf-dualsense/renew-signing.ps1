# Run elevated after build.ps1 has staged a replacement certificate/package.
# Existing install.ps1 Update deliberately forbids changing signer in place.
# Retire the owned source and trust first, then install the staged package.
param()
$ErrorActionPreference = 'Stop'
$prototypeRoot = $PSScriptRoot
$results = Join-Path $prototypeRoot 'results'
$package = Join-Path $prototypeRoot 'package'
$backup = Join-Path (Split-Path (Split-Path (Split-Path $prototypeRoot -Parent) -Parent) -Parent) '..\outputs\RaijuVhfPrototype-0.1.0.1\package'
$backup = [IO.Path]::GetFullPath($backup)
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
if (!([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Certificate rotation needs an elevated PowerShell.' }
if (Get-Process -Name destiny2,raiju-bridge,raiju-vhf-probe,raiju-tray -ErrorAction SilentlyContinue) { throw 'Close Destiny and both bridges before replacing the signer.' }
$before=Get-Content -LiteralPath (Join-Path $results 'certificate-rotation-before.json') -Raw | ConvertFrom-Json
$manifest=Get-Content -LiteralPath (Join-Path $package 'manifest.json') -Raw | ConvertFrom-Json
if ($manifest.certificateThumbprint -eq $before.oldCertificateThumbprint) { throw 'No replacement signer was staged.' }
foreach ($entry in $manifest.files.PSObject.Properties) {
    if ($entry.Name -notin @('RaijuVhf.dll','RaijuVhf.inf','RaijuVhf.cat','RaijuVhf.cer','raiju-vhf-probe.exe','raiju-tray.exe')) { throw 'Unexpected package filename' }
    if ((Get-FileHash -LiteralPath (Join-Path $package $entry.Name)).Hash -ne $entry.Value) { throw ('Package hash mismatch: '+$entry.Name) }
}
$backupManifest=Get-Content -LiteralPath (Join-Path $backup 'manifest.json') -Raw | ConvertFrom-Json
if ($backupManifest.certificateThumbprint -ne $before.oldCertificateThumbprint) { throw 'Rollback snapshot has the wrong signer.' }
foreach ($entry in $backupManifest.files.PSObject.Properties) {
    if ($entry.Name -notin @('RaijuVhf.dll','RaijuVhf.inf','RaijuVhf.cat','RaijuVhf.cer','raiju-vhf-probe.exe')) { throw 'Unexpected rollback filename' }
    if ((Get-FileHash -LiteralPath (Join-Path $backup $entry.Name)).Hash -ne $entry.Value) { throw ('Rollback hash mismatch: '+$entry.Name) }
}
try {
    & (Join-Path $prototypeRoot 'install.ps1') -Action Remove
    & (Join-Path $prototypeRoot 'install.ps1') -Action Install
    $device=@(Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'ROOT\*' -and $_.HardwareID -contains 'ROOT\RAIJUVHFPROTOTYPE' })
    if ($device.Count -ne 1) { throw 'Expected exactly one installed prototype source.' }
    $version=(Get-PnpDeviceProperty -InstanceId $device[0].InstanceId -KeyName 'DEVPKEY_Device_DriverVersion').Data
    if ($version -ne $manifest.driverVersion) { throw 'Windows did not activate the re-signed package.' }
    foreach ($store in @('Root','TrustedPublisher')) {
        if (Test-Path -LiteralPath "Cert:\LocalMachine\$store\$($before.oldCertificateThumbprint)") { throw 'Old signer trust was not retired.' }
        if (!(Test-Path -LiteralPath "Cert:\LocalMachine\$store\$($manifest.certificateThumbprint)")) { throw 'New signer trust is missing.' }
    }
    [ordered]@{result='PASS';oldCertificateThumbprint=$before.oldCertificateThumbprint;newCertificateThumbprint=$manifest.certificateThumbprint;certificateExpires=$manifest.certificateExpires;driverVersion=$version;oldTrustRemoved=$true;time=(Get-Date).ToUniversalTime().ToString('o')} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $results 'certificate-rotation-result.json')
} catch {
    $originalError=$_.Exception.Message
    try {
        & (Join-Path $prototypeRoot 'install.ps1') -Action Remove
        foreach ($name in @('RaijuVhf.dll','RaijuVhf.pdb','RaijuVhf.inf','RaijuVhf.cat','RaijuVhf.cer','raiju-vhf-probe.exe','manifest.json')) { Copy-Item -LiteralPath (Join-Path $backup $name) -Destination $package -Force }
        & (Join-Path $prototypeRoot 'install.ps1') -Action Install
        $rolledBack=$true
    } catch { $rolledBack=$false; $rollbackError=$_.Exception.Message }
    [ordered]@{result='FAIL';error=$originalError;rolledBack=$rolledBack;rollbackError=$rollbackError;time=(Get-Date).ToUniversalTime().ToString('o')} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $results 'certificate-rotation-result.json')
    throw $originalError
}
