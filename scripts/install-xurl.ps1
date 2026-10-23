$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$revision = '24eedb2ff2b141b025efa1a1658fd7f3e88cdd01'
$destination = Join-Path $PWD 'target\runtime-support\xurl'
cargo install --git https://github.com/Xuanwo/xurl --rev $revision --locked --root $destination --force xurl-cli
if ($LASTEXITCODE -ne 0) { throw '构建 xURL 失败' }
Invoke-WebRequest -UseBasicParsing "https://raw.githubusercontent.com/Xuanwo/xurl/$revision/LICENSE" -OutFile "$destination\LICENSE"
[IO.File]::WriteAllText("$destination\REVISION", "$revision`n")
& "$destination\bin\xurl.exe" --version
if ($LASTEXITCODE -ne 0) { throw '验证 xURL 失败' }
