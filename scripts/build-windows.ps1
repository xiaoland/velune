# Builds Velune for Windows into target\windows\Velune: velune.exe, the
# RelayKit DLL it loads, and the runtime helper resources.
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not $env:GO) { $env:GO = (Get-Command go).Source }
if (-not $env:VELUNE_RELAYKIT_CC) { $env:VELUNE_RELAYKIT_CC = (Get-Command x86_64-w64-mingw32-clang).Source }
$support = 'target\runtime-support'
if (-not (Test-Path "$support\node_modules\huihua") -or -not (Test-Path "$support\THIRD_PARTY_NOTICES.md")) {
    throw 'Runtime support is missing; install it as docs/development.md describes.'
}
cargo build --locked --release -p velune-windows
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$app = 'target\windows\Velune'
Remove-Item -Recurse -Force $app -ErrorAction SilentlyContinue
$resources = "$app\resources"
New-Item -ItemType Directory -Force "$resources\ThirdParty" | Out-Null
Copy-Item target\release\velune.exe, target\release\velune_relaykit.dll $app
$scripts = 'pi_sdk', 'pi_sessions', 'pi_rpc', 'pi_virtual_model', 'pi_auth', 'pi_provider_import', 'huihua_sessions'
foreach ($script in $scripts) { Copy-Item "packages\agent-runtime\resources\$script.mjs" $resources }
Copy-Item -Recurse "$support\node_modules" "$resources\node_modules"
Copy-Item packages\agent-runtime\runtime-support\THIRD_PARTY_NOTICES.md "$resources\ThirdParty"
Copy-Item -Recurse packages\agent-runtime\runtime-support\licenses "$resources\ThirdParty\licenses"
Copy-Item packages\agent-runtime\runtime-support\package-lock.json "$resources\ThirdParty\runtime-support-package-lock.json"
Copy-Item -Recurse app\mac\Licenses\RelayKit "$resources\ThirdParty\RelayKit"
Copy-Item packages\ai-provider\relaykit-bridge\go.mod, packages\ai-provider\relaykit-bridge\go.sum "$resources\ThirdParty\RelayKit"
Write-Output "Built $((Resolve-Path $app).Path)"
