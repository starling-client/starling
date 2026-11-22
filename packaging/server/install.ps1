# Installs sunrise-server (docs/server.md) on Windows: by default the current Sunrise Nightly (GitHub's latest release
# while there is none), or the release -Version names, downloaded from GitHub Releases and checked against the release's
# signed SHA256SUMS (docs/release-flow.md, "Signed checksums") by sunrise-verify, a small program this script
# downloads and checks against the SHA-256 pinned below before it runs it. A download that can't be verified is not
# installed.
#
#   irm https://raw.githubusercontent.com/sunrise-org/sunrise/main/packaging/server/install.ps1 | iex
#   & ([scriptblock]::Create((irm .../install.ps1))) -Version 26.3-0.1.0 -Dir D:\Sunrise
#
# A stable release's tag (26.3-0.1.0) puts the server on that stable line, which `sunrise-server update` then follows.
#
# Releases go to <Dir>\releases\<version>, <Dir>\current points at the one in use, and <Dir>\bin (added to
# your PATH) holds sunrise-server.cmd and sunrise-prepare.cmd. The game files you prepare from your own
# Minecraft stay in <Dir>\game-files across updates. Nothing of Mojang's is downloaded. No admin rights.
param(
    [string]$Version = "latest",
    [string]$Dir = (Join-Path $env:LOCALAPPDATA "Sunrise"),
    # For CI fixtures only (never in the docs): GitHub's own SHA-256 is all that is checked.
    [switch]$InsecureChecksumOnly
)
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Repo = "sunrise-org/sunrise"
# sunrise-verify, the release checker the install pins: its SHA-256 per platform,
# from release verifier@$VerifierVersion. "unpinned" until the first verifier release exists.
# >>> verifier pin (tools/ci/pin_verifier.py rewrites this block; do not edit it by hand)
$VerifierVersion = '1.0.0'
$VerifierSha256 = @{
    'linux-x86_64' = '310bba30dd6dba7147d9a14692c4efb5887875c6d6eca953a5e888106f5a7630'
    'linux-aarch64' = '5e293dbc2552db7a7df79105005425463b7b0eef2d97ff30a8e65d156f0c50b6'
    'macos-arm64' = '0b934c07fe4003bc25554a95adef2dfc3a3b73e84b9647e100d0a3acd472e11e'
    'macos-x64' = '912223491295e60f2d338a141b47fe2deab161608b1b8b02134ca7b7bdf0430b'
    'windows-x64' = 'd26077550c2b8695db181d3f32ba1cb7f2f22cff455f78d6148a298df56f9b79'
    'windows-arm64' = 'ef5bbdf1440c303c0e8c6056159fc46166727257f1e6a1b98ef63def7932d1d9'
}
# <<< verifier pin
$Fallback = $null
$Architecture = $env:PROCESSOR_ARCHITEW6432
if (-not $Architecture) { $Architecture = $env:PROCESSOR_ARCHITECTURE }
switch ($Architecture) {
    "AMD64" { $Platform = "windows-x64"; $Asset = "sunrise-windows-x64.zip" }
    # A release without an ARM64 build installs the x64 one, which Windows runs under emulation.
    "ARM64" { $Platform = "windows-arm64"; $Asset = "sunrise-windows-arm64.zip"; $Fallback = "sunrise-windows-x64.zip" }
    default { throw "No Sunrise release for Windows on $Architecture yet: build it from source (docs/server.md)" }
}

function Get-Release([string]$Path) {
    Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/$Path" -Headers @{ Accept = "application/vnd.github+json" }
}
if ($Version -eq "latest") {
    try { $Release = Get-Release "tags/nightly" } catch { $Release = Get-Release "latest" }
} else {
    $Release = Get-Release "tags/$Version"
}
$Tag = $Release.tag_name
$RawTag = $Tag
if ($Tag -ceq "dev" -or $Tag.StartsWith("dev-")) {
    throw "$Tag is a Dev build, made to test main: servers don't install it. Install the Nightly (the default) or a stable release (-Version <tag>)."
}
# A Nightly installs under its date and commit, as `sunrise-server update` names it:
# "Nightly 2026-10-28 <middle dot> 0c02067" is nightly-2026-10-28-0c02067 ([char]0xB7: this file stays ASCII).
# Its signature is checked for exactly that version, so the title must name it (as
# sunrise_release_sign::build_title_version reads it, strictly).
$Nightly = $null
if ($Tag -eq "nightly" -or $Tag.StartsWith("nightly-")) {
    if ($Release.name -cnotmatch "^Nightly (\d{4}-\d{2}-\d{2}) $([char]0xB7) ([0-9a-f]{7,40})$") {
        throw "This Nightly's title doesn't name its date and commit as Sunrise signs them, so it can't be checked: not installed"
    }
    $Nightly = "nightly-$($Matches[1])-$($Matches[2])"
    $Tag = $Nightly
}
$Item = $Release.assets | Where-Object { $_.name -eq $Asset } | Select-Object -First 1
if (-not $Item -and $Fallback) {
    Write-Host "Release $Tag has no ${Asset}: installing $Fallback (x64, under emulation)"
    $Asset = $Fallback
    $Item = $Release.assets | Where-Object { $_.name -eq $Asset } | Select-Object -First 1
}
if (-not $Item) { throw "Release $Tag has no $Asset" }
$Digest = $null
if ($Item.digest -and $Item.digest.StartsWith("sha256:")) { $Digest = $Item.digest.Substring(7) }

$Tmp = Join-Path ([IO.Path]::GetTempPath()) ("sunrise-install-" + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
    # A Nightly is checked against SHA256SUMS.v2.sig, signed for its own version (nightly-<date>-<commit>, from its
    # title), never the channel's SHA256SUMS.sig: an older Nightly can't be passed off as the current one
    # (docs/security.md, "Nightly signatures").
    if ($Nightly) { $SigTag = $Nightly; $SigFile = "SHA256SUMS.v2.sig" } else { $SigTag = $RawTag; $SigFile = "SHA256SUMS.sig" }
    $Files = Join-Path $Tmp "files"
    New-Item -ItemType Directory -Path $Files | Out-Null
    $Zip = Join-Path $Files $Asset
    if ($InsecureChecksumOnly) {
        Write-Warning "-InsecureChecksumOnly: the release signature is NOT checked, only the SHA-256 GitHub publishes for the download. Anyone who can change the release can change that too. This is for test fixtures, not for servers."
        if (-not $Digest) { throw "Release $Tag publishes no SHA-256 for $Asset" }
        Write-Host "Downloading Sunrise $Tag ($Asset)..."
        Invoke-WebRequest -Uri $Item.browser_download_url -OutFile $Zip
        $Got = (Get-FileHash -Algorithm SHA256 $Zip).Hash.ToLowerInvariant()
        if ($Got -ne $Digest.ToLowerInvariant()) {
            throw "$Asset doesn't match its SHA-256 (got $Got, expected $Digest): not installed"
        }
    } else {
        # The verifier: the pinned version's binary for this platform, run only when its SHA-256 is the pinned one.
        # The pin is the trust anchor (tools/ci/pin_verifier.py); nothing outside this file can change it.
        $Pin = $VerifierSha256[$Platform]
        $VerifierFrom = $VerifierVersion
        if (-not $VerifierFrom -or $VerifierFrom -eq "unpinned") {
            throw "This script has no pinned sunrise-verify yet, so it can't check the release signature: not installed. Use the install.ps1 from Sunrise's main branch."
        }
        if (-not $Pin -or $Pin -eq "unpinned") { throw "There is no sunrise-verify for $Platform (the signature check needs it): not installed" }
        $Verifier = Join-Path $Tmp "sunrise-verify.exe"
        try {
            Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/$Repo/releases/download/verifier%40$VerifierFrom/sunrise-verify-$Platform.exe" -OutFile $Verifier
        } catch {
            throw "Could not download sunrise-verify $VerifierFrom for $Platform (needed to check the release signature): not installed"
        }
        $Got = (Get-FileHash -Algorithm SHA256 $Verifier).Hash.ToLowerInvariant()
        if ($Got -ne $Pin.ToLowerInvariant()) {
            throw "sunrise-verify for $Platform isn't the pinned build (got $Got, expected $Pin): not installed. The download may have been tampered with; please report it on Discord or GitHub."
        }

        $SumsItem = $Release.assets | Where-Object { $_.name -eq "SHA256SUMS" } | Select-Object -First 1
        $SigItem = $Release.assets | Where-Object { $_.name -eq $SigFile } | Select-Object -First 1
        if (-not $SumsItem) {
            throw "Release $Tag has no SHA256SUMS (its checksum list), so it can't be checked: not installed"
        } elseif (-not $SigItem -and $Nightly) {
            throw "$Tag has no $SigFile (its signature for this exact build): it was published before Nightlies were signed for their own version, so it can't be checked: not installed. Install the current Nightly (-Version latest) instead."
        } elseif (-not $SigItem) {
            throw "Release $Tag has no $SigFile (the signature of its checksum list), so it can't be checked: not installed"
        }
        try {
            Invoke-WebRequest -UseBasicParsing -Uri $SumsItem.browser_download_url -OutFile (Join-Path $Files "SHA256SUMS")
            Invoke-WebRequest -UseBasicParsing -Uri $SigItem.browser_download_url -OutFile (Join-Path $Files $SigFile)
        } catch { throw "Could not download SHA256SUMS and ${SigFile}: not installed" }
        Write-Host "Downloading Sunrise $Tag ($Asset)..."
        try { Invoke-WebRequest -Uri $Item.browser_download_url -OutFile $Zip } catch { throw "Could not download ${Asset}: not installed" }
        # The same check the launcher and `sunrise-server update` make, with the keys built into the verifier. Only an
        # exit code of 0 from this very run passes: the code from any earlier program is cleared first, and a verifier
        # that fails to start leaves none.
        $global:LASTEXITCODE = $null
        $Code = $null
        $ErrorActionPreference = "Continue"
        try {
            $Said = (& $Verifier verify --tag $SigTag --signature $SigFile --only $Asset $Files 2>&1 | Out-String).Trim()
            $Code = $global:LASTEXITCODE
        } catch {
            $Said = "sunrise-verify could not run: $($_.Exception.Message)"
        } finally {
            $ErrorActionPreference = "Stop"
        }
        if ($Code -isnot [int] -or $Code -ne 0) {
            throw "The release check failed ($SigFile for $SigTag), so $Tag was not installed: $Said. The download may have been tampered with; please report it on Discord or GitHub."
        }
        Write-Host "Release signature OK (Sunrise release key)."
    }
    $Unpacked = Join-Path $Tmp "x"
    Expand-Archive -Path $Zip -DestinationPath $Unpacked
    $Server = Get-ChildItem -Path $Unpacked -Recurse -Filter "sunrise-server.exe" | Select-Object -First 1
    if (-not $Server) { throw "$Asset holds no sunrise-server.exe (a release from before it existed?)" }

    $Releases = Join-Path $Dir "releases"
    $GameFiles = Join-Path $Dir "game-files\sunrise-client"
    $Bin = Join-Path $Dir "bin"
    New-Item -ItemType Directory -Force -Path $Releases, $GameFiles, $Bin | Out-Null
    $Target = Join-Path $Releases $Tag
    if (Test-Path $Target) { Remove-Item -Recurse -Force $Target }
    Move-Item -Path $Server.Directory.FullName -Destination $Target
    # Prepared game files live outside the release, so updates and rollbacks keep them. Junctions need no
    # admin rights, unlike symbolic links.
    $Local = Join-Path $Target ".local"
    New-Item -ItemType Directory -Force -Path $Local | Out-Null
    New-Item -ItemType Junction -Path (Join-Path $Local "sunrise-client") -Target $GameFiles | Out-Null
    $Current = Join-Path $Dir "current"
    if (Test-Path $Current) { (Get-Item $Current).Delete() }
    New-Item -ItemType Junction -Path $Current -Target $Target | Out-Null
    foreach ($Tool in "sunrise-server", "sunrise-prepare") {
        Set-Content -Path (Join-Path $Bin "$Tool.cmd") -Encoding ASCII -Value "@`"$Current\$Tool.exe`" %*"
    }
    $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (-not (($UserPath -split ";") -contains $Bin)) {
        [Environment]::SetEnvironmentVariable("Path", ($UserPath.TrimEnd(";") + ";" + $Bin).TrimStart(";"), "User")
        Write-Host "Added $Bin to your PATH: open a new terminal to use it."
    }
    Write-Host "Installed Sunrise $Tag in $Target."
    if (-not (Get-ChildItem -Path $GameFiles -ErrorAction SilentlyContinue)) {
        Write-Host "Next, prepare the game files from your own Minecraft 26.3 (it needs Java 25 and the game installed"
        Write-Host "once by the official launcher; see docs/server.md):"
        Write-Host "  sunrise-prepare --prepare --server-jar C:\path\to\server.jar"
    }
    Write-Host "Then set up a server: mkdir my-server; cd my-server; sunrise-server init"
} finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
