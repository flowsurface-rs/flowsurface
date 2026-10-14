param(
    [string]$Executable = "target/packaging/win-signed/flowsurface.exe",
    [string]$Publisher,
    [string]$Version,
    [string]$Output = "target/packaging/flowsurface-x86_64-windows.msix"
)
$ErrorActionPreference = "Stop"
Set-Location "$PSScriptRoot/.."
$Executable = (Resolve-Path $Executable).Path
if (!$Publisher) {
    $signature = Get-AuthenticodeSignature $Executable
    if ($signature.Status -ne 'Valid') { throw "A valid signed EXE or explicit test Publisher is required" }
    $Publisher = $signature.SignerCertificate.Subject
}
if (!$Version) {
    $metadata = cargo metadata --no-deps --locked --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "Cargo metadata failed" }
    $Version = ($metadata.packages | Where-Object name -eq flowsurface).version
}
if ($Version -notmatch '^(\d+)\.(\d+)\.(\d+)$') { throw "MSIX requires a numeric major.minor.patch version: $Version" }
if (@($Matches[1], $Matches[2], $Matches[3]) | Where-Object { [long]$_ -gt 65535 }) { throw "MSIX version components must be at most 65535" }
$packageVersion = "$Version.0"
$sdk = Get-ChildItem "${env:ProgramFiles(x86)}/Windows Kits/10/bin/*/x64/makeappx.exe" | Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1
if (!$sdk) { throw "MakeAppx.exe not found; install the Windows 10/11 SDK" }
$makepri = Join-Path $sdk.Directory.FullName "makepri.exe"
if (!(Test-Path $makepri)) { throw "MakePri.exe not found; install the Windows 10/11 SDK" }
$stage = "target/packaging/msix-layout"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force "$stage/Assets" | Out-Null
Copy-Item $Executable "$stage/flowsurface.exe"
$iconSizes = @{
    "Square44x44Logo.png" = 44
    "Square44x44Logo.scale-200.png" = 88
    "Square44x44Logo.scale-400.png" = 176
    "Square150x150Logo.png" = 150
    "Square150x150Logo.scale-200.png" = 300
    "Square150x150Logo.scale-400.png" = 600
    "StoreLogo.png" = 50
}
foreach ($size in @(16, 20, 24, 30, 32, 36, 40, 48, 60, 64, 72, 80, 96, 256)) {
    foreach ($form in @("", "_altform-unplated", "_altform-lightunplated")) {
        $iconSizes["Square44x44Logo.targetsize-$size$form.png"] = $size
    }
}
Add-Type -AssemblyName System.Drawing
$icon = [System.Drawing.Image]::FromFile("$PWD/assets/app-icon/windows.png")
try {
    foreach ($name in $iconSizes.Keys) {
        $size = $iconSizes[$name]
        $bitmap = [System.Drawing.Bitmap]::new($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
                $graphics.DrawImage($icon, [System.Drawing.Rectangle]::new(0, 0, $size, $size))
            } finally { $graphics.Dispose() }
            $bitmap.Save("$PWD/$stage/Assets/$name", [System.Drawing.Imaging.ImageFormat]::Png)
        } finally { $bitmap.Dispose() }
    }
} finally { $icon.Dispose() }
[xml]$manifest = Get-Content scripts/packaging/windows/AppxManifest.xml
$manifest.Package.Identity.Publisher = $Publisher
$manifest.Package.Identity.Version = $packageVersion
$manifest.Save("$PWD/$stage/AppxManifest.xml")
$priConfig = "$PWD/target/packaging/msix-priconfig.xml"
& $makepri createconfig /cf $priConfig /dq en-US /o
if ($LASTEXITCODE -ne 0) { throw "MakePri configuration failed" }
& $makepri new /pr "$PWD/$stage" /cf $priConfig /mn "$PWD/$stage/AppxManifest.xml" /of "$PWD/$stage/resources.pri" /o
if ($LASTEXITCODE -ne 0) { throw "MakePri resource indexing failed" }
New-Item -ItemType Directory -Force (Split-Path $Output) | Out-Null
& $sdk.FullName pack /d $stage /p $Output /o
if ($LASTEXITCODE -ne 0) { throw "MakeAppx packaging failed" }
