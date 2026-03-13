param(
  [string]$Version = "latest",
  [string]$InstallDir = "$HOME\\.local\\bin",
  [switch]$VerifyChecksums = $false
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
$checksumUrl = "https://github.com/$Repo/releases/download/$Version/lensmap-${Version}-checksums.txt"

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

  if ($VerifyChecksums) {
    $checksumPath = Join-Path $tmp "$($asset)-checksum.txt"
    try {
      Invoke-WebRequest -Uri $checksumUrl -OutFile $checksumPath
      $escapedAsset = [regex]::Escape($asset)
      $expected = (Get-Content -Path $checksumPath | Where-Object {
        $_ -match "^([a-fA-F0-9]{64})\s+.*$escapedAsset$"
      }) |
        Select-Object -First 1
      $expected = ($expected -split '\s+')[0]
      if (-not $expected) {
        throw "Unable to find checksum entry for $asset in release checksum file."
      }
      $actual = (Get-FileHash -Path $zipPath -Algorithm SHA256).Hash.ToLower()
      if ($actual -ne $expected.ToLower()) {
        throw "Checksum mismatch for $asset."
      }
    }
    catch {
      throw $_
    }
  }
  Expand-Archive -Path $zipPath -DestinationPath $tmp -Force

  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
  Copy-Item (Join-Path $tmp "lensmap.exe") (Join-Path $InstallDir "lensmap.exe") -Force
  Write-Host "Installed lensmap to $InstallDir\\lensmap.exe"
}
finally {
  Remove-Item -Path $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
