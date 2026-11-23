# Runs the Rust test suite under cargo-llvm-cov and prints line coverage.
$ErrorActionPreference = "Stop"
Push-Location "$PSScriptRoot/../rust"
try {
  cargo llvm-cov nextest --workspace --cobertura --output-path target/coverage-rust.xml
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
  $cov = ([xml](Get-Content target/coverage-rust.xml)).coverage
  $pct = [math]::Round([double]$cov.'line-rate' * 100, 2)
  Write-Output "Rust line coverage: $pct% ($($cov.'lines-covered')/$($cov.'lines-valid'))"
} finally {
  Pop-Location
}
