# Builds Velune for Windows into target\windows\Velune: Velune.exe (WinUI 3,
# self-contained), the bindings and RelayKit DLLs it loads, and the runtime
# helper resources.
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
if (-not $env:GO) { $env:GO = (Get-Command go).Source }
if (-not $env:VELUNE_RELAYKIT_CC) { $env:VELUNE_RELAYKIT_CC = (Get-Command x86_64-w64-mingw32-clang).Source }
$support = 'target\runtime-support'
if (-not (Test-Path "$support\node_modules\huihua") -or -not (Test-Path "$support\THIRD_PARTY_NOTICES.md")) {
    throw 'Runtime support is missing; install it as docs/development.md describes.'
}
cargo build --locked --release -p velune-bindings
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

# The C# bindings are generated from the metadata in the library just built,
# by uniffi-bindgen-cs at the commit that supports UniFFI 0.32 (NordSecurity
# PR #176); cargo keeps an installed copy at the same commit.
$generator = 'target\tools\uniffi-bindgen-cs'
cargo install uniffi-bindgen-cs --locked --quiet --root $generator `
    --git https://github.com/NordSecurity/uniffi-bindgen-cs --rev cf0aa7a8fb59358a846708730a58a02bf206ac05
if ($LASTEXITCODE -ne 0) { throw 'installing uniffi-bindgen-cs failed' }
# The generator runs `cargo metadata`; outside cargo it finds cargo only by CARGO.
$env:CARGO = (Get-Command cargo).Source
& "$generator\bin\uniffi-bindgen-cs.exe" --library target\release\velune_bindings.dll `
    --config packages\bindings\uniffi.toml --out-dir target\bindings\cs --no-format
if ($LASTEXITCODE -ne 0) { throw 'uniffi-bindgen-cs failed' }

$app = 'target\windows\Velune'
Remove-Item -Recurse -Force $app -ErrorAction SilentlyContinue
dotnet publish app\windows-winui\Velune.csproj --configuration Release --output $app
if ($LASTEXITCODE -ne 0) { throw 'dotnet publish failed' }
Copy-Item target\release\velune_bindings.dll, target\release\velune_relaykit.dll $app

$resources = "$app\resources"
New-Item -ItemType Directory -Force "$resources\ThirdParty" | Out-Null
$scripts = 'pi_sdk', 'pi_sessions', 'pi_rpc', 'pi_virtual_model', 'pi_auth', 'pi_provider_import', 'huihua_sessions'
foreach ($script in $scripts) { Copy-Item "packages\agent-runtime\resources\$script.mjs" $resources }
Copy-Item -Recurse "$support\node_modules" "$resources\node_modules"
Copy-Item packages\agent-runtime\runtime-support\THIRD_PARTY_NOTICES.md "$resources\ThirdParty"
Copy-Item -Recurse packages\agent-runtime\runtime-support\licenses "$resources\ThirdParty\licenses"
Copy-Item packages\agent-runtime\runtime-support\package-lock.json "$resources\ThirdParty\runtime-support-package-lock.json"
Copy-Item -Recurse app\mac\Licenses\RelayKit "$resources\ThirdParty\RelayKit"
Copy-Item packages\ai-provider\relaykit-bridge\go.mod, packages\ai-provider\relaykit-bridge\go.sum "$resources\ThirdParty\RelayKit"
Write-Output "Built $((Resolve-Path $app).Path)"
