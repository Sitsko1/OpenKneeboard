---
parent: Internals
---

# Automated Tests

OpenKneeboard has two test suites:

* **C++**: a GoogleTest executable, `OpenKneeboard-Tests` (`src/tests/`). Most tests are *characterization* tests: they pin current behavior, including quirks, so that the Rust port can be checked against it. A test that exposes a bug still asserts the current behavior, and the bug goes in the "Known quirks" list of `docs/internals/rust-port-parity-contract.md`.
* **Rust**: a Cargo workspace at `rust/`.

The C++ tests are only built with `-DOPENKNEEBOARD_BUILD_TESTS=ON`. The default build is unchanged and does not need GoogleTest.

## Prerequisites

* `git submodule update --init third-party/vcpkg`
* the MSVC v143 toolset. With Visual Studio 2026, add the component `Microsoft.VisualStudio.Component.VC.14.44.17.14.x86.x64` and configure with `-G "Visual Studio 18 2026" -T v143`.
* [OpenCppCoverage](https://github.com/OpenCppCoverage/OpenCppCoverage), for C++ coverage (`winget install OpenCppCoverage.OpenCppCoverage`)
* Rust: `rustup` (the toolchain version is pinned in `rust/rust-toolchain.toml`), then `cargo install cargo-nextest cargo-llvm-cov --locked`
* **Close OpenKneeboard before running the tests.** The SHM tests use the same named shared memory as a locally built OpenKneeboard. If they detect a running instance, they skip with a message rather than fail.

## Running the tests

From a VS Developer PowerShell (or with VS's bundled `cmake`/`ctest` on `PATH`):

```powershell
cmake -B build-tests --preset vcpkg -DOPENKNEEBOARD_BUILD_TESTS=ON
cmake --build build-tests --config Debug --target OpenKneeboard-Tests
ctest --test-dir build-tests -C Debug --output-on-failure

cd rust
cargo nextest run
cargo clippy --all-targets
cargo fmt --check
```

## Coverage

```powershell
scripts/coverage-cpp.ps1    # -BuildDir build-tests -Config Debug
scripts/coverage-rust.ps1
```

`coverage-cpp.ps1` writes `build-tests/coverage-cpp.xml` (Cobertura). It fails if the report covers zero lines: OpenCppCoverage needs `--cover_children` because `ctest` spawns the test process, and without it the report silently measures nothing.

CI runs both suites with coverage: the C++ suite in the `Build (Debug)` job and the Rust suite in the `Rust` job. Each job uploads its Cobertura report as an artifact (`coverage-cpp`, `coverage-rust`) and writes the summary line to the job summary.

## Baseline

Measured 2026-11-22 at commit `1930c053`.

| Measure | Value |
|---|---|
| Pre-Phase-0 automated tests | 0 (0% coverage) |
| C++ line coverage, `src/` (OpenCppCoverage, modules loaded by tests) | 34.58% (546/1579) |
| C++ line coverage of all `src/` C++ lines (546 ÷ 43,982 lines in `src/**/*.cpp`, excluding `src/tests`) | 1.24% |
| Rust line coverage | n/a (0/0): `okb-shm` holds only constants; its test is in `tests/` |
