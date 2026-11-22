# Runs the C++ test suite under OpenCppCoverage and prints line coverage.
# Exits non-zero if tests fail or if the report covers zero lines.
param(
  [string]$BuildDir = "build-tests",
  [string]$Config = "Debug"
)
$ErrorActionPreference = "Stop"
$repo = (Resolve-Path "$PSScriptRoot/..").Path
$BuildDir = (Resolve-Path $BuildDir).Path
$report = Join-Path $BuildDir "coverage-cpp.xml"

# --cover_children: ctest spawns the test executable; without this the
# report silently covers nothing.
OpenCppCoverage `
  --sources "$repo\src" `
  --excluded_sources "$repo\src\tests" `
  --excluded_sources "$repo\third-party" `
  --cover_children `
  --export_type "cobertura:$report" `
  -- ctest --test-dir $BuildDir -C $Config --output-on-failure
$testExit = $LASTEXITCODE

$cov = ([xml](Get-Content $report)).coverage
$pct = [math]::Round([double]$cov.'line-rate' * 100, 2)
Write-Output "C++ line coverage: $pct% ($($cov.'lines-covered')/$($cov.'lines-valid'))"

if ([int]$cov.'lines-covered' -eq 0) {
  Write-Error "Coverage report has zero covered lines; is --cover_children working?"
  exit 1
}
exit $testExit
