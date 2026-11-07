# Development defaults to Rust dev / .NET Debug. Release remains an explicit
# delivery build, and neither mode installs or launches the application.
param(
    [Alias('Profile')] [ValidateSet('dev', 'release')] [string] $BuildProfile = 'dev'
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$python = (Get-Command python).Source
if (-not $env:GO) { $env:GO = (Get-Command go).Source }
if (-not $env:VELUNE_RELAYKIT_CC) { $env:VELUNE_RELAYKIT_CC = (Get-Command x86_64-w64-mingw32-clang).Source }
# PowerShell 5 otherwise sends ASCII to a native process's stdin.
$OutputEncoding = [System.Text.UTF8Encoding]::new($false)
if ($env:VELUNE_BUILD_MANAGED -ne 'windows') {
    & $python -I scripts/build-support.py run --platform windows --profile $BuildProfile -- `
        powershell -NoProfile -ExecutionPolicy Bypass -File $PSCommandPath -Profile $BuildProfile
    exit $LASTEXITCODE
}
$versionInfo = & $python -I scripts/version.py --format build-info
if ($LASTEXITCODE -ne 0) { throw 'product version or VELUNE_BUILD_NUMBER is invalid' }
if (-not $versionInfo -or $versionInfo.Split("`t").Count -ne 3) { throw 'scripts/version.py returned invalid build metadata' }
function Invoke-BuildSupport {
    & $python -I scripts/build-support.py @args
    if ($LASTEXITCODE -ne 0) { throw "build-support failed: $args" }
}
$support = 'target\runtime-support'
if (-not (Test-Path "$support\node_modules\huihua") -or -not (Test-Path "$support\THIRD_PARTY_NOTICES.md")) {
    throw 'Runtime support is missing; install it as docs/development.md describes.'
}
foreach ($file in 'bin\xurl.exe', 'LICENSE', 'REVISION') {
    if (-not (Test-Path "$support\xurl\$file")) { throw '缺少 xURL；请先运行 scripts\install-xurl.ps1。' }
}
$nativeProfile = if ($BuildProfile -eq 'dev') { 'debug' } else { 'release' }
$configuration = if ($BuildProfile -eq 'dev') { 'Debug' } else { 'Release' }
$app = if ($BuildProfile -eq 'dev') { 'target\windows-dev\Velune' } else { 'target\windows\Velune' }
$native = "target\host-rust\$nativeProfile"
cargo build --locked --profile $BuildProfile --target-dir target/host-rust -p velune-bindings --lib
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

# An installed generator is a fixed host tool, independent of application source.
# Adopt Cargo's exact installation receipt once; normal builds need no git fetch.
$revision = 'cf0aa7a8fb59358a846708730a58a02bf206ac05'
$generator = 'target\tools\uniffi-bindgen-cs'
$generatorBinary = "$generator\bin\uniffi-bindgen-cs.exe"
$generatorMarker = "$generator\.velune-generator.json"
$generatorReady = $false
if (Test-Path $generatorBinary) {
    $binaryHash = (Get-FileHash -Algorithm SHA256 $generatorBinary).Hash.ToLowerInvariant()
    if (Test-Path $generatorMarker) {
        $installed = Get-Content -Raw -Encoding UTF8 $generatorMarker | ConvertFrom-Json
        $generatorReady = $installed.revision -eq $revision -and $installed.sha256 -eq $binaryHash
    }
    if (-not $generatorReady -and (Test-Path "$generator\.crates2.json")) {
        $receipt = Get-Content -Raw -Encoding UTF8 "$generator\.crates2.json" | ConvertFrom-Json
        $source = "^uniffi-bindgen-cs [^ ]+ \(git\+https://github\.com/NordSecurity/uniffi-bindgen-cs\?rev=$revision#$revision\)$"
        $generatorReady = @($receipt.installs.PSObject.Properties | Where-Object { $_.Name -match $source -and $_.Value.bins -contains 'uniffi-bindgen-cs.exe' }).Count -gt 0
    }
}
if (-not $generatorReady) {
    cargo install uniffi-bindgen-cs --locked --quiet --root $generator `
        --git https://github.com/NordSecurity/uniffi-bindgen-cs --rev $revision
    if ($LASTEXITCODE -ne 0) { throw 'installing uniffi-bindgen-cs failed' }
    $binaryHash = (Get-FileHash -Algorithm SHA256 $generatorBinary).Hash.ToLowerInvariant()
}
@{ revision = $revision; sha256 = $binaryHash } | ConvertTo-Json -Compress |
    & $python -I scripts/build-support.py write $generatorMarker
if ($LASTEXITCODE -ne 0) { throw 'recording uniffi-bindgen-cs identity failed' }
# The generator runs cargo metadata. Each crate retains its own namespace config.
$env:CARGO = (Get-Command cargo).Source
Invoke-BuildSupport bindgen --library "$native\velune_bindings.dll" --language cs `
    --generator $generatorBinary --out-dir target/bindings/cs

# Restore from the checked-in dependency graph, then prevent publish from
# resolving a different graph after the lock has been checked.
dotnet restore app\windows\Velune.csproj --locked-mode --runtime win-x64
if ($LASTEXITCODE -ne 0) { throw 'dotnet locked restore failed' }
# MSBuild retains its normal input tracking; generated C# timestamps only change
# when its contents do. The publish tree is not deleted between invocations.
dotnet publish app\windows\Velune.csproj --configuration $configuration --output $app --no-restore
if ($LASTEXITCODE -ne 0) { throw 'dotnet publish failed' }
Invoke-BuildSupport copy "$native\velune_bindings.dll" "$app\velune_bindings.dll"
Invoke-BuildSupport copy "$native\velune_relaykit.dll" "$app\velune_relaykit.dll"

$resources = "$app\resources"
Invoke-BuildSupport copy LICENSE "$resources\ThirdParty\LICENSE"
$scripts = 'pi_sdk', 'pi_sessions', 'pi_rpc', 'pi_virtual_model', 'pi_background_work', 'pi_auth', 'pi_provider_import', 'huihua_sessions'
foreach ($script in $scripts) { Invoke-BuildSupport copy "packages\agent-runtime\resources\$script.mjs" "$resources\$script.mjs" }
if (Test-Path $resources) {
    Get-ChildItem $resources -Filter '*.mjs' -File | Where-Object { $_.BaseName -notin $scripts } | Remove-Item
}
Invoke-BuildSupport sync "$support\node_modules" "$resources\node_modules" --prune
Invoke-BuildSupport sync "$support\xurl" "$resources\xurl" --prune --include 'bin/*' --include LICENSE --include REVISION
Remove-Item "$resources\xurl\.crates.toml", "$resources\xurl\.crates2.json" -ErrorAction SilentlyContinue
Invoke-BuildSupport sync packages/agent-runtime/runtime-support/licenses "$resources\ThirdParty\licenses" --prune
Invoke-BuildSupport copy packages/agent-runtime/runtime-support/package-lock.json "$resources\ThirdParty\runtime-support-package-lock.json"
Invoke-BuildSupport sync app/mac/Licenses/RelayKit "$resources\ThirdParty\RelayKit" --prune --include '*.txt' --include '*.md'
foreach ($file in 'go.mod', 'go.sum') {
    Invoke-BuildSupport copy "packages\ai-provider\relaykit-bridge\$file" "$resources\ThirdParty\RelayKit\$file"
}
foreach ($file in 'LICENSE', 'REVISION') {
    Invoke-BuildSupport copy "$support\xurl\$file" "$resources\ThirdParty\xurl\$file"
}
# An incrementally assembled notice must not append a second copy on every run.
$notice = (Get-Content -Raw -Encoding UTF8 THIRD_PARTY_NOTICES.md).TrimEnd() + "`n`n## Runtime-support JavaScript dependencies`n`n" + (Get-Content -Raw -Encoding UTF8 packages/agent-runtime/runtime-support/THIRD_PARTY_NOTICES.md).TrimEnd() + @'


## xURL

本应用包含 Xuanwo 与贡献者开发的官方 xURL CLI，未修改其源码。
来源：https://github.com/Xuanwo/xurl
许可证：Apache-2.0。完整许可证与固定源码提交见 xurl/。
'@
$notice | & $python -I scripts/build-support.py write "$resources\ThirdParty\THIRD_PARTY_NOTICES.md"
if ($LASTEXITCODE -ne 0) { throw 'writing third-party notice failed' }
Invoke-BuildSupport sync app/windows/Licenses "$resources\ThirdParty\OpenTelemetry" --prune --include 'OpenTelemetry-*'
$nugetNotices = "$resources\ThirdParty\NuGet"
& $python -I scripts/dotnet-licenses.py app/windows/obj/project.assets.json $nugetNotices
if ($LASTEXITCODE -ne 0) { throw 'collecting NuGet licence notices failed' }
Invoke-BuildSupport copy app/windows/packages.lock.json "$nugetNotices\packages.lock.json"
$licensePending = "$resources\ThirdParty\rust-licenses.txt.pending"
& $python -I scripts/android-licenses.py local-runtime,remote $licensePending x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw 'collecting Rust licence notices failed' }
Invoke-BuildSupport copy $licensePending "$resources\ThirdParty\rust-licenses.txt"
Remove-Item $licensePending
Write-Output "Built $((Resolve-Path $app).Path) ($BuildProfile)"
