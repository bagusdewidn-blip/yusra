# Sekali setelah clone (Windows): aktifkan hook repo.
$ErrorActionPreference = "Stop"
Set-Location (git rev-parse --show-toplevel)
git config core.hooksPath .githooks
Write-Host "core.hooksPath = .githooks"
Write-Host ("Identitas commit: {0} <{1}>" -f (git config user.name), (git config user.email))
