param(
  [string]$Version = "latest",
  [string]$InstallDir = "$HOME\\.local\\bin"
)

$ErrorActionPreference = "Stop"
$Repo = "protheuslabs/Lensmap"

if ($Version -eq "latest") {
  $release = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest"
  $Version = $release.tag_name
}

if (-not $Version) {
  throw "Unable to resolve release version"
}

$arch = if ($env:PROCESSOR_ARCHITECTURE -match "ARM64") { "aarch64" } else { "x86_64" }
$os = "pc-windows-msvc"
$asset = "lensmap-$Version-$arch-$os.zip"
$url = "https://github.com/$Repo/releases/download/$Version/$asset"

$tmp = Join-Path $env:TEMP ("lensmap-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null

try {
  $zipPath = Join-Path $tmp $asset
  try {
    Invoke-WebRequest -Uri $url -OutFile $zipPath
  }
  catch {
    throw "No prebuilt asset found for $arch-$os at $Version. Build from source with 'cargo build --release'."
  }
  Expand-Archive -Path $zipPath -DestinationPath $tmp -Force

  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
  Copy-Item (Join-Path $tmp "lensmap.exe") (Join-Path $InstallDir "lensmap.exe") -Force
  Write-Host "Installed lensmap to $InstallDir\\lensmap.exe"
}
finally {
  Remove-Item -Path $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
