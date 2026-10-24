param(
    [ValidateSet('Auto','CUDA','CPU')][string]$Backend = 'Auto',
    [string]$Destination = (Join-Path (Split-Path $PSScriptRoot -Parent) '.cache/runtime')
)
$ErrorActionPreference = 'Stop'
$Destination=[System.IO.Path]::GetFullPath($Destination)
if ($Backend -eq 'Auto') {
    $Backend = if (Get-Command nvidia-smi.exe -ErrorAction SilentlyContinue) { 'CUDA' } else { 'CPU' }
}
$Manifest = Get-Content (Join-Path $PSScriptRoot 'runtime-manifest.json') -Raw | ConvertFrom-Json
$ManifestHash=(Get-FileHash (Join-Path $PSScriptRoot 'runtime-manifest.json') -Algorithm SHA256).Hash.ToLowerInvariant()
$Existing=Join-Path $Destination 'runtime.json'
if(Test-Path $Existing){
    $Saved=Get-Content $Existing -Raw|ConvertFrom-Json
    if($Saved.manifest_sha256 -eq $ManifestHash -and $Saved.backend -eq $Backend -and $Saved.llama -eq $Manifest.llama.version -and $Saved.teleocr -eq $Manifest.teleocr.version){
        $Valid=$true
        foreach($File in $Saved.files.PSObject.Properties){$Path=Join-Path $Destination $File.Name;if(-not(Test-Path $Path) -or (Get-FileHash $Path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $File.Value){$Valid=$false;break}}
        if($Valid){Write-Host "Reusing verified runtime: $Backend";return}
    }
}
$Cache = Join-Path (Split-Path $Destination -Parent) 'downloads'
New-Item -ItemType Directory -Force -Path $Cache | Out-Null
function Get-VerifiedArchive($Asset) {
    $Path = Join-Path $Cache $Asset.name
    if ((Test-Path $Path) -and (Get-FileHash $Path -Algorithm SHA256).Hash.ToLowerInvariant() -eq $Asset.sha256) { return $Path }
    & curl.exe --fail --location --retry 4 --output "$Path.part" $Asset.url
    if ($LASTEXITCODE -ne 0) { throw "Download failed: $($Asset.name)" }
    if ((Get-FileHash "$Path.part" -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Asset.sha256) { throw "SHA256 mismatch: $($Asset.name)" }
    Move-Item -Force "$Path.part" $Path
    return $Path
}
$Stage = "$Destination.tmp-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force -Path "$Stage/bin/teleocr","$Stage/licenses","$Stage/unpack" | Out-Null
try {
    $Names = if ($Backend -eq 'CUDA') { @('llama-b11747-bin-win-cuda-13.4-x64.zip','cudart-llama-bin-win-cuda-13.4-x64.zip') } else { @('llama-b11747-bin-win-cpu-x64.zip') }
    $RuntimePattern = '^(llama-server(?:-impl)?\.exe|llama-server-impl\.dll|llama-common\.dll|llama\.dll|mtmd\.dll|ggml(?:-base|-cuda|-cpu-[^.]*)?\.dll|cublas(?:Lt)?64_[^.]+\.dll|cudart64_[^.]+\.dll|libomp[^.]*\.dll)$'
    foreach ($Name in $Names) {
        $Asset = $Manifest.llama.assets.$Name
        $Asset | Add-Member -NotePropertyName name -NotePropertyValue $Name -Force
        $Archive = Get-VerifiedArchive $Asset
        $Unpack = Join-Path "$Stage/unpack" $Name
        Expand-Archive -LiteralPath $Archive -DestinationPath $Unpack -Force
        Get-ChildItem $Unpack -Recurse -File | Where-Object { $_.Name -match $RuntimePattern } | ForEach-Object { Copy-Item $_.FullName "$Stage/bin/" }
        Get-ChildItem $Unpack -Recurse -File | Where-Object { $_.Name -match '^(LICENSE|COPYING|NOTICE|EULA)' } | ForEach-Object { Copy-Item $_.FullName (Join-Path "$Stage/licenses" "$Name-$($_.Name)") }
    }
    $Archive = Get-VerifiedArchive $Manifest.teleocr.asset
    & tar.exe -xf $Archive -C "$Stage/unpack"
    if ($LASTEXITCODE -ne 0) { throw 'TeleOCR extraction failed' }
    $Tele = Get-ChildItem "$Stage/unpack" -Recurse -File -Filter teleocr.exe | Select-Object -First 1
    if (-not $Tele) { throw 'Missing teleocr.exe' }
    Copy-Item $Tele.FullName "$Stage/bin/teleocr/teleocr.exe"
    # BlinkOCR imports images; the optional PDFium renderer is not required.
    foreach ($Required in @('llama-server.exe','llama-server-impl.dll','llama.dll','mtmd.dll','ggml.dll','ggml-base.dll','teleocr/teleocr.exe')) {
        if (-not (Test-Path (Join-Path "$Stage/bin" $Required))) { throw "Missing runtime file: $Required" }
    }
    & "$Stage/bin/llama-server.exe" --version
    if ($LASTEXITCODE -ne 0) { throw 'llama-server startup failed' }
    & "$Stage/bin/teleocr/teleocr.exe" --version
    if ($LASTEXITCODE -ne 0) { throw 'TeleOCR startup failed' }
    Remove-Item "$Stage/unpack" -Recurse -Force
    $Files = @{}
    Get-ChildItem $Stage -Recurse -File | ForEach-Object { $Relative = $_.FullName.Substring($Stage.Length + 1).Replace('\','/'); $Files[$Relative] = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    @{manifest_sha256=$ManifestHash;backend=$Backend;llama=$Manifest.llama.version;teleocr=$Manifest.teleocr.version;files=$Files} | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 "$Stage/runtime.json"
    $Previous="$Destination.previous"
    if(Test-Path $Previous){Remove-Item $Previous -Recurse -Force}
    if(Test-Path $Destination){Move-Item $Destination $Previous}
    try {Move-Item $Stage $Destination} catch {if(Test-Path $Previous){Move-Item $Previous $Destination};throw}
    if(Test-Path $Previous){Remove-Item $Previous -Recurse -Force}
    Write-Host "Runtime prepared: $Backend -> $Destination"
} finally {
    if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
}
