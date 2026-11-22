# Builds the Rust okb-ffi staticlib (CMake target okb_ffi) for the C++ tests.
include(FetchContent)

FetchContent_Declare(
  Corrosion
  URL "https://github.com/corrosion-rs/corrosion/archive/refs/tags/v0.6.1.zip"
  URL_HASH "SHA256=5c1c5c6ab7196fe4466321c14e9698ec2d0a977c89fd9f1639a37fcffdf604f2"
  EXCLUDE_FROM_ALL
)

# Corrosion resolves the rustup toolchain from CMake's working directory, which
# doesn't see rust/rust-toolchain.toml; pin it explicitly.
file(
  STRINGS "${CMAKE_SOURCE_DIR}/rust/rust-toolchain.toml" RUST_TOOLCHAIN_CHANNEL
  REGEX "^channel = \"[^\"]+\"$"
)
string(REGEX MATCH "\"([^\"]+)\"" _ "${RUST_TOOLCHAIN_CHANNEL}")
set(Rust_TOOLCHAIN "${CMAKE_MATCH_1}")

FetchContent_MakeAvailable(Corrosion)

# cargo runs in rust/, so rust/.cargo/config.toml applies (+crt-static).
corrosion_import_crate(MANIFEST_PATH "${CMAKE_SOURCE_DIR}/rust/Cargo.toml" CRATES okb-ffi)
