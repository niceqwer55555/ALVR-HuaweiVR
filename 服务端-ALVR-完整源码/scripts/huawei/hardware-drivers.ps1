#requires -Version 5.1
<#
    hardware-drivers.ps1 - install / report the Windows device drivers used by
    the Huawei VR Glass (the devices listed in Device Manager).

    Three device classes are involved:
      * DisplayPort monitor        (DISPLAY\RTK2560 / HUAWEI GLASS CV10)
      * USB composite device       (USB\VID_12D1&PID_107E, IMU / keys / control)
      * Virtual display driver     (optional, some firmware revisions)

    The INF packages are the copies shipped next to this script in
    `hw-usb-driver\` (taken from the official Huawei PC VR assistant).

    Usage:
      powershell -File hardware-drivers.ps1 -Status            # report only
      powershell -File hardware-drivers.ps1 -Install           # needs admin
      powershell -File hardware-drivers.ps1 -Install -BaseDir "D:\ALVR"

    Status mode prints ASCII KEY=VALUE lines so the dashboard can parse them.
    ASCII-only on purpose: PowerShell 5.1 mis-reads UTF-8 without BOM.
#>
[CmdletBinding()]
param(
    [switch]$Status,
    [switch]$Install,
    [string]$BaseDir = ''
)

$ErrorActionPreference = 'Continue'

# --- resolve base / driver root without hardcoding any drive letter ---------
$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $BaseDir) { $BaseDir = $scriptDir }
$BaseDir = (Resolve-Path -LiteralPath $BaseDir).Path

$roots = @(
    (Join-Path $BaseDir 'hw-usb-driver'),
    (Join-Path $BaseDir '..\deps\huawei-usb-driver'),
    (Join-Path $BaseDir '..\..\deps\huawei-usb-driver'),
    (Join-Path $BaseDir '..\..\..\deps\huawei-usb-driver'),
    (Join-Path $scriptDir '..\hw-usb-driver'),
    (Join-Path $scriptDir '..\..\deps\huawei-usb-driver')
)
$driverRoot = $roots |
    Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Container) } |
    Select-Object -First 1
if ($driverRoot) { $driverRoot = (Resolve-Path -LiteralPath $driverRoot).Path }

function Get-HuaweiDevices {
    Get-PnpDevice -ErrorAction SilentlyContinue |
        Where-Object {
            $_.InstanceId -match 'VID_12D1&PID_107' -or
            $_.InstanceId -match 'RTK2560' -or
            $_.InstanceId -match 'VDD' -or
            $_.FriendlyName -match 'HUAWEI' -or
            $_.FriendlyName -match 'VR Glass' -or
            $_.FriendlyName -match 'Virtual Display'
        }
}

function Write-Status {
    $all = @(Get-PnpDevice -ErrorAction SilentlyContinue)
    $disp = @($all | Where-Object { $_.Class -eq 'Monitor' -and ($_.InstanceId -match 'RTK2560' -or $_.InstanceId -match 'HWV' -or $_.InstanceId -match 'CV10') })
    $usb = @($all | Where-Object { $_.InstanceId -match 'VID_12D1&PID_107' })
    $hid = @($all | Where-Object { $_.Class -eq 'HIDClass' -and $_.InstanceId -match 'VID_12D1&PID_107' })
    $vdd = @($all | Where-Object { $_.InstanceId -match 'VDD' -or $_.FriendlyName -match 'Virtual Display' })
    $bad = @($all | Where-Object {
        $_.Status -ne 'OK' -and (
            $_.InstanceId -match 'VID_12D1&PID_107' -or $_.InstanceId -match 'RTK2560' -or
            $_.FriendlyName -match 'HUAWEI' -or $_.FriendlyName -match 'VR Glass'
        )
    })

    Write-Output ('DISPLAY_COUNT=' + $disp.Count)
    Write-Output ('USB_COUNT=' + $usb.Count)
    Write-Output ('USB_PROBLEM=' + @($usb | Where-Object { $_.Status -ne 'OK' }).Count)
    Write-Output ('HID_COUNT=' + $hid.Count)
    Write-Output ('VDD_COUNT=' + $vdd.Count)
    Write-Output ('PROBLEM_COUNT=' + $bad.Count)
    if ($driverRoot) { Write-Output ('DRIVER_ROOT=' + $driverRoot) }
    else { Write-Output 'DRIVER_ROOT=' }

    foreach ($dev in $bad) {
        Write-Output ('PROBLEM_DEVICE=' + $dev.FriendlyName + ' | ' + $dev.Status + ' | ' + $dev.InstanceId)
    }
}

if ($Status -or (-not $Install)) {
    Write-Status
    exit 0
}

# --- install ---------------------------------------------------------------
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Output 'ERROR=NOT_ADMIN'
    Write-Status
    Write-Host 'Requires Administrator: right-click -> Run as administrator, or use the dashboard button.' -ForegroundColor Red
    exit 2
}

if (-not $driverRoot) {
    Write-Output 'ERROR=DRIVER_ROOT_NOT_FOUND'
    exit 3
}

$pnputil = Join-Path $env:WINDIR 'System32\pnputil.exe'
if (-not (Test-Path -LiteralPath $pnputil -PathType Leaf)) {
    Write-Output 'ERROR=PNPUTIL_NOT_FOUND'
    exit 4
}

# Only the Huawei-specific packages. Do NOT install the generic mtp/cdc-acm
# INFs: they would replace inbox Windows drivers for unrelated phones.
$wanted = @(
    'hw_cdcacm.inf', 'hw_cdcmdm.inf', 'hw_qumdm.inf', 'hw_quser.inf',
    'hw_quusbnet.inf', 'hw_goadb.inf', 'hw_gohdb.inf', 'hw_usbdev.inf',
    'ew_usbccgpfilter.inf', 'VirtualDisplayDriver.inf', 'virtual_display_device.inf'
)

$infDirs = @(
    (Join-Path $driverRoot 'IMU\Driver\X64'),
    (Join-Path $driverRoot 'driverX64')
)

$infs = @()
foreach ($dir in $infDirs) {
    if (-not (Test-Path -LiteralPath $dir -PathType Container)) { continue }
    foreach ($name in $wanted) {
        $candidate = Join-Path $dir $name
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { $infs += Get-Item -LiteralPath $candidate }
    }
}

Write-Host "driver root : $driverRoot"
Write-Host "INF packages: $($infs.Count)"

foreach ($inf in $infs) {
    Write-Host "--- pnputil /add-driver $($inf.Name) /install"
    & $pnputil /add-driver $inf.FullName /install 2>&1 | ForEach-Object { Write-Host "    $_" }
}

& $pnputil /scan-devices 2>&1 | ForEach-Object { Write-Host "    $_" }

Write-Status
exit 0
