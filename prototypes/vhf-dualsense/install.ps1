param([ValidateSet('Install','Update','Remove')][string]$Action = 'Install')
$ErrorActionPreference = 'Stop'
$prototypeRoot = $PSScriptRoot
$package = Join-Path $prototypeRoot 'package'
$results = Join-Path $prototypeRoot 'results'
New-Item -ItemType Directory -Path $results -Force | Out-Null
$statePath = Join-Path $results 'install-state.json'
$logPath = Join-Path $results 'install.log'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
if (!([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Run this script in an elevated PowerShell. It does not change boot or code-integrity settings.' }
Start-Transcript -LiteralPath $logPath -Append | Out-Null
try {
    $devcon = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Tools\10.0.26100.0\x64\devcon.exe'
    if (!(Test-Path -LiteralPath $devcon)) { throw 'The WDK devcon tool used for this local prototype is missing.' }
    $devices = @(Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -like 'ROOT\*' -and $_.HardwareID -contains 'ROOT\RAIJUVHFPROTOTYPE' })
    if ($Action -eq 'Remove') {
        $tray = Join-Path $package 'raiju-tray.exe'
        if (Test-Path -LiteralPath $tray) {
            if (Get-Process -Name raiju-tray -ErrorAction SilentlyContinue) {
                Start-Process -FilePath $tray -ArgumentList '--quit' -WindowStyle Hidden -Wait
                Get-Process -Name raiju-tray -ErrorAction SilentlyContinue | Wait-Process -Timeout 10
            }
            $startupRemoval = Start-Process -FilePath $tray -ArgumentList '--remove-startup' -WindowStyle Hidden -Wait -PassThru
            if ($startupRemoval.ExitCode) { throw 'Could not remove tray login startup' }
        }
        $state = if (Test-Path -LiteralPath $statePath) { Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json } else { $null }
        [string[]]$infs = if ($state) { @($state.driverPackages | Where-Object { $_ -match '^oem[0-9]+\.inf$' }) } else { @() }
        foreach ($device in $devices) {
            $oem = (Get-PnpDeviceProperty -InstanceId $device.InstanceId -KeyName 'DEVPKEY_Device_DriverInfPath' -ErrorAction SilentlyContinue).Data
            if ($oem -match '^oem[0-9]+\.inf$') { $infs += $oem }
            & $devcon remove ('@' + $device.InstanceId)
            if ($LASTEXITCODE -notin @(0,1)) { throw "Could not remove $($device.InstanceId)" }
        }
        foreach ($oem in ($infs | Select-Object -Unique)) {
            & pnputil /delete-driver $oem
            if ($LASTEXITCODE -notin @(0,3010)) { throw "Could not delete prototype package $oem" }
        }
        if ($state) {
            foreach ($store in @('Root','TrustedPublisher')) {
                if ($store -in @($state.addedStores)) {
                    $certPath = "Cert:\LocalMachine\$store\$($state.certificateThumbprint)"
                    if (Test-Path -LiteralPath $certPath) { Remove-Item -LiteralPath $certPath }
                }
            }
        }
        [ordered]@{removed=$true; time=(Get-Date).ToUniversalTime().ToString('o')} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $results 'remove-result.json')
        Write-Output 'Prototype device and its added certificate trust removed.'
        return
    }
    if (Get-Process -Name destiny2,raiju-bridge,raiju-tray,raiju-vhf-probe -ErrorAction SilentlyContinue) { throw 'Close Destiny 2 and both Raiju bridges before installing this prototype.' }
    if ($Action -eq 'Install' -and $devices.Count) { throw 'Prototype device already exists. Use Update for a rebuilt package.' }
    if ($Action -eq 'Update' -and $devices.Count -ne 1) { throw 'Update requires exactly one existing prototype source device.' }
    if (Test-Path -LiteralPath $statePath) {
        $oldState = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json
        foreach ($store in @($oldState.addedStores)) {
            if ($Action -eq 'Install' -and (Test-Path -LiteralPath "Cert:\LocalMachine\$store\$($oldState.certificateThumbprint)")) { throw 'An earlier prototype trust record remains. Run Remove first.' }
        }
    }
    $manifest = Get-Content -LiteralPath (Join-Path $package 'manifest.json') -Raw | ConvertFrom-Json
    foreach ($entry in $manifest.files.PSObject.Properties) {
        if ($entry.Name -notin @('RaijuVhf.dll','RaijuVhf.inf','RaijuVhf.cat','RaijuVhf.cer','raiju-vhf-probe.exe','raiju-tray.exe')) { throw 'Unexpected manifest filename' }
        if ((Get-FileHash -LiteralPath (Join-Path $package $entry.Name) -Algorithm SHA256).Hash -ne $entry.Value) { throw "Package hash mismatch: $($entry.Name)" }
    }
    $certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new((Join-Path $package 'RaijuVhf.cer'))
    if ($certificate.Subject -ne 'CN=Raiju VHF Prototype Local Test' -or $certificate.Thumbprint -ne $manifest.certificateThumbprint) { throw 'Unexpected prototype certificate' }
    foreach ($name in @('RaijuVhf.dll','RaijuVhf.cat')) {
        $signature = Get-AuthenticodeSignature -LiteralPath (Join-Path $package $name)
        if ($signature.SignerCertificate.Thumbprint -ne $certificate.Thumbprint) { throw "Unexpected signer: $name" }
    }
    if ($Action -eq 'Update' -and (!$oldState -or $oldState.certificateThumbprint -ne $certificate.Thumbprint)) { throw 'Update must retain the recorded signing certificate. Remove first to change signer.' }
    [string[]]$added = if ($Action -eq 'Update') { @($oldState.addedStores) } else { @() }
    [string[]]$knownPackages = if ($Action -eq 'Update') { @($oldState.driverPackages | Where-Object { $_ -match '^oem[0-9]+\.inf$' }) } else { @() }
    foreach ($existing in $devices) {
        $oldInf = (Get-PnpDeviceProperty -InstanceId $existing.InstanceId -KeyName 'DEVPKEY_Device_DriverInfPath' -ErrorAction SilentlyContinue).Data
        if ($oldInf -match '^oem[0-9]+\.inf$') { $knownPackages += $oldInf }
    }
    foreach ($store in @('Root','TrustedPublisher')) {
        if (!(Test-Path -LiteralPath "Cert:\LocalMachine\$store\$($certificate.Thumbprint)")) {
            Import-Certificate -FilePath (Join-Path $package 'RaijuVhf.cer') -CertStoreLocation "Cert:\LocalMachine\$store" | Out-Null
            $added += $store
        }
        [ordered]@{certificateThumbprint=$certificate.Thumbprint;addedStores=$added;driverPackages=@($knownPackages | Select-Object -Unique)} | ConvertTo-Json | Set-Content -LiteralPath $statePath
    }
    foreach ($name in @('RaijuVhf.dll','RaijuVhf.cat')) {
        if ((Get-AuthenticodeSignature -LiteralPath (Join-Path $package $name)).Status -ne 'Valid') { throw "Signature validation failed: $name" }
    }
    $verb = if ($Action -eq 'Update') {'update'} else {'install'}
    & $devcon $verb (Join-Path $package 'RaijuVhf.inf') 'ROOT\RAIJUVHFPROTOTYPE'
    $installExit = $LASTEXITCODE
    if ($installExit -eq 1) { throw 'Windows requested a reboot. No reboot was started; inspect results/install.log.' }
    if ($installExit -ne 0) { throw "Driver install failed: $installExit. No boot/signature settings were changed." }
    Start-Sleep -Seconds 2
    $device = Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -like 'ROOT\*' -and $_.HardwareID -contains 'ROOT\RAIJUVHFPROTOTYPE' }
    if (!$device -or $device.Status -ne 'OK') { throw 'Prototype device did not start. Inspect its problem code and results/install.log.' }
    $newInf = (Get-PnpDeviceProperty -InstanceId $device.InstanceId -KeyName 'DEVPKEY_Device_DriverInfPath').Data
    if ($newInf -match '^oem[0-9]+\.inf$') { $knownPackages += $newInf }
    [ordered]@{certificateThumbprint=$certificate.Thumbprint;addedStores=$added;driverPackages=@($knownPackages | Select-Object -Unique)} | ConvertTo-Json | Set-Content -LiteralPath $statePath
    & (Join-Path $package 'raiju-vhf-probe.exe') status
    if ($LASTEXITCODE) { throw 'Driver interface health check failed' }
    [ordered]@{installed=$true;instanceId=$device.InstanceId;status=$device.Status;time=(Get-Date).ToUniversalTime().ToString('o')} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $results 'install-result.json')
    $tray = Join-Path $package 'raiju-tray.exe'
    if ('raiju-tray.exe' -in $manifest.files.PSObject.Properties.Name) {
        $startupRegistration = Start-Process -FilePath $tray -ArgumentList '--register-startup' -WindowStyle Hidden -Wait -PassThru
        if ($startupRegistration.ExitCode) { throw 'Could not register tray login startup' }
        Write-Output 'Driver installed. Open package/raiju-tray.exe to start now; it also starts at your next login.'
    } else {
        Write-Output 'UMDF source is ready. It creates no HID controller until a client is run.'
    }
} catch {
    [ordered]@{action=$Action;error=$_.Exception.Message;time=(Get-Date).ToUniversalTime().ToString('o')} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $results 'install-error.json')
    throw
} finally { Stop-Transcript | Out-Null }
