# Builds the Rust crates: okb-xr-layer (the OpenXR layer cdylib, CMake target
# okb_xr_layer) for the product, and okb-ffi (staticlib, okb_ffi) for the C++ tests.
include(FetchContent)
scoped_include(system.cmake) # FXC_EXE

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

if (BUILD_IS_32BIT)
  set(Rust_CARGO_TARGET "i686-pc-windows-msvc")
endif ()

FetchContent_MakeAvailable(Corrosion)

# cargo runs in rust/, so rust/.cargo/config.toml applies (+crt-static).
corrosion_import_crate(
  MANIFEST_PATH "${CMAKE_SOURCE_DIR}/rust/Cargo.toml"
  CRATES okb-ffi okb-xr-layer
)
if (NOT OPENKNEEBOARD_BUILD_TESTS)
  set_target_properties(cargo-build_okb_ffi PROPERTIES EXCLUDE_FROM_ALL ON)
endif ()

corrosion_set_env_vars(
  okb_xr_layer
  "OKB_OPENXR_LAYER_NAME=${PROJECT_OPENXR_API_LAYER_NAME}"
  "OKB_OPENXR_LAYER_DESCRIPTION=${PROJECT_OPENXR_API_LAYER_DESCRIPTION}"
  "OKB_OPENXR_LAYER_IMPL_VERSION=${OPENXR_API_LAYER_IMPLEMENTATION_VERSION}"
  "OKB_REVERSE_DOMAIN=${PROJECT_REVERSE_DOMAIN}"
  "OKB_VERSION_MAJOR=${CMAKE_PROJECT_VERSION_MAJOR}"
  "OKB_VERSION_MINOR=${CMAKE_PROJECT_VERSION_MINOR}"
  "OKB_VERSION_PATCH=${CMAKE_PROJECT_VERSION_PATCH}"
  "OKB_VERSION_BUILD=${VERSION_BUILD}"
  "OKB_FXC=${FXC_EXE}"
)

# The Rust standard library and crates statically linked into the layer DLLs.
include(ok_add_license_file)
ok_add_license_file(LICENSE-Rust-crates.txt "LICENSE-ThirdParty-Rust-crates.txt")
