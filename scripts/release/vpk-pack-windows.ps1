# Pack PwrRoute for Windows with vpk: Setup.exe, a per-user MSI, the
# portable zip and the `win` update feed. release.yml's windows-sign job runs
# it from the signing input (with release notes and Azure signing);
# preview-build.yml runs it from the checkout (unsigned). One script, so the
# preview packs what the release packs.
#
# - --runtime win-x64: without it vpk defaults to x86 Setup.exe and Update.exe.
# - --instLocation PerUser: browser registration is HKCU, and vpk's default
#   dual-scope MSI preselects per-machine in its dialog.
# - --azureTrustedSignFile: vpk signs pwrroute.exe, Update.exe and the stub,
#   then Setup.exe and the MSI, through Azure Artifact Signing; the dlib reads
#   the service principal from AZURE_TENANT_ID / AZURE_CLIENT_ID /
#   AZURE_CLIENT_SECRET.
# - --icon: packaging/windows/PwrRoute.ico (from scripts/release/make-icns.sh),
#   for Setup.exe, the MSI and the Start menu shortcut; pwrroute.exe itself
#   carries no icon resource yet.
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)] [string]$Version,
  [Parameter(Mandatory = $true)] [string]$PackDir,
  [Parameter(Mandatory = $true)] [string]$OutputDir,
  [Parameter(Mandatory = $true)] [string]$Icon,
  [string]$ReleaseNotes,
  [string]$AzureSignFile
)

$ErrorActionPreference = "Stop"

$vpkArgs = @(
  "--skip-updates", "pack",
  "--packId", "PwrRoute",
  "--packVersion", $Version,
  "--runtime", "win-x64",
  "--packDir", $PackDir,
  "--mainExe", "pwrroute.exe",
  "--packTitle", "PwrRoute",
  "--packAuthors", "PwrDrvr LLC",
  "--shortcuts", "StartMenuRoot",
  "--icon", $Icon,
  "--msi", "--instLocation", "PerUser",
  "--outputDir", $OutputDir
)
if ($ReleaseNotes) { $vpkArgs += @("--releaseNotes", $ReleaseNotes) }
if ($AzureSignFile) { $vpkArgs += @("--azureTrustedSignFile", $AzureSignFile) }

& vpk @vpkArgs
if ($LASTEXITCODE -ne 0) { throw "vpk pack failed (exit code $LASTEXITCODE)." }
