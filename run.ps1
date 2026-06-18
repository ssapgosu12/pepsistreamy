# pepsistreamy 실행 런처 (Windows PowerShell)
# 사용: 우클릭 > PowerShell로 실행  또는  ./run.ps1 [doctor|devices|meter|run]
# 첫 실행 시 .\.venv 가상환경을 만들고 의존성을 설치합니다.

$ErrorActionPreference = "Stop"
Set-Location -Path $PSScriptRoot

# 한국어/기호 출력이 깨지지 않도록 콘솔을 UTF-8로
try { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 } catch {}
$env:PYTHONUTF8 = "1"

$venv = Join-Path $PSScriptRoot ".venv"
$py = Join-Path $venv "Scripts\python.exe"

if (-not (Test-Path $py)) {
    Write-Host "[pepsistreamy] 가상환경 생성 중..." -ForegroundColor Cyan
    python -m venv $venv
    & $py -m pip install --upgrade pip
    & $py -m pip install -r (Join-Path $PSScriptRoot "requirements.txt")
}

$cmd = if ($args.Count -ge 1) { $args[0] } else { "run" }
$rest = if ($args.Count -gt 1) { $args[1..($args.Count - 1)] } else { @() }

& $py -m pepsistreamy $cmd @rest
