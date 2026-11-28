$ErrorActionPreference = 'Stop'
& python -I (Join-Path $PSScriptRoot 'install-xurl.py')
exit $LASTEXITCODE
