ok_add_library(detours-ext STATIC detours-ext.cpp)
target_link_libraries(detours-ext PUBLIC ThirdParty::detours OpenKneeboard-dprint)

configure_file(
  OpenKneeboard-OpenXR.in.def
  OpenKneeboard-OpenXR${BUILD_BITNESS}.def
  @ONLY
  NEWLINE_STYLE UNIX
)
ok_add_library(
  OpenKneeboard-OpenXR
  MODULE
  DUALARCH
  OpenKneeboard-OpenXR${BUILD_BITNESS}.def
  OpenXRKneeboard.cpp
  OpenXRD3D11Kneeboard.cpp
  OpenXRD3D12Kneeboard.cpp
  OpenXRVulkanKneeboard.cpp
  OpenXRNext.cpp
)
target_link_libraries(
  OpenKneeboard-OpenXR
  PRIVATE
  ThirdParty::OpenXR
  ThirdParty::DirectXTK
  ThirdParty::DirectXTK12
  ThirdParty::VulkanHeaders
  OpenKneeboard-D3D11
  OpenKneeboard-D3D12
  OpenKneeboard-Vulkan
  OpenKneeboard-Elevation
  OpenKneeboard-RayIntersectsRect
  OpenKneeboard-SHM
  OpenKneeboard-SHM-Client-D3D11
  OpenKneeboard-SHM-Client-D3D12
  OpenKneeboard-SHM-Client-Vulkan
  OpenKneeboard-SpriteAtlas
  OpenKneeboard-StateMachine
  OpenKneeboard-VRKneeboard
  OpenKneeboard-config
  OpenKneeboard-dprint
  OpenKneeboard-scope_exit
  OpenKneeboard-shims
  OpenKneeboard-version
)

configure_file(
  OpenKneeboard-OpenXR.in.json
  OpenKneeboard-OpenXR.intermediate.json
  NEWLINE_STYLE UNIX
)

# Keep the un-suffixed name to keep compatibility with previous installations
set(64BIT_OPENXR_JSON "${BUILD_OUT_BINDIR}/OpenKneeboard-OpenXR.json")
set(32BIT_OPENXR_JSON "${BUILD_OUT_BINDIR}/OpenKneeboard-OpenXR32.json")
if (BUILD_IS_32BIT)
  set(OPENXR_JSON_OUT "${32BIT_OPENXR_JSON}")
else ()
  set(OPENXR_JSON_OUT "${64BIT_OPENXR_JSON}")
endif ()
file(
  GENERATE
  OUTPUT "${OPENXR_JSON_OUT}"
  INPUT "${CMAKE_CURRENT_BINARY_DIR}/OpenKneeboard-OpenXR.intermediate.json"
  NEWLINE_STYLE UNIX
)

# The Rust port of the OpenXR layer (rust/crates/okb-xr-layer, built by
# Corrosion as okb_xr_layer), shipped next to the C++ layer under its own names.
set(64BIT_RUST_OPENXR_DLL "${BUILD_OUT_BINDIR}/OpenKneeboard-OpenXR-Rust64.dll")
set(32BIT_RUST_OPENXR_DLL "${BUILD_OUT_BINDIR}/OpenKneeboard-OpenXR-Rust32.dll")
if (BUILD_IS_32BIT)
  set(RUST_OPENXR_DLL "${32BIT_RUST_OPENXR_DLL}")
  set(RUST_OPENXR_JSON "${BUILD_OUT_BINDIR}/OpenKneeboard-OpenXR-Rust32.json")
else ()
  set(RUST_OPENXR_DLL "${64BIT_RUST_OPENXR_DLL}")
  set(RUST_OPENXR_JSON "${BUILD_OUT_BINDIR}/OpenKneeboard-OpenXR-Rust.json")
endif ()
cmake_path(GET RUST_OPENXR_DLL STEM RUST_OPENXR_STEM)
add_custom_target(
  OpenKneeboard-OpenXR-Rust
  ALL
  COMMAND "${CMAKE_COMMAND}" -E make_directory "${BUILD_OUT_BINDIR}" "${BUILD_OUT_PDBDIR}"
  COMMAND
  "${CMAKE_COMMAND}" -E copy_if_different
  "$<TARGET_FILE:okb_xr_layer-shared>"
  "${RUST_OPENXR_DLL}"
  COMMAND
  "${CMAKE_COMMAND}" -E copy_if_different
  "$<TARGET_FILE_DIR:okb_xr_layer-shared>/okb_xr_layer.pdb"
  "${BUILD_OUT_PDBDIR}/${RUST_OPENXR_STEM}.pdb"
  BYPRODUCTS "${RUST_OPENXR_DLL}" "${BUILD_OUT_PDBDIR}/${RUST_OPENXR_STEM}.pdb"
  VERBATIM
)
add_dependencies(OpenKneeboard-OpenXR-Rust cargo-build_okb_xr_layer)
set_target_properties(OpenKneeboard-OpenXR-Rust PROPERTIES OUTPUT_FILE "${RUST_OPENXR_DLL}")
add_dependencies(runtime-dependencies OpenKneeboard-OpenXR-Rust)

# Mirrors DUALARCH: the 32-bit sub-build's `build32` builds the 32-bit layer,
# and the 64-bit build depends on `build32` through OpenKneeboard-OpenXR-Rust32.
if (BUILD_IS_32BIT)
  add_dependencies(build32 OpenKneeboard-OpenXR-Rust)
else ()
  add_custom_target(OpenKneeboard-OpenXR-Rust32)
  add_dependencies(OpenKneeboard-OpenXR-Rust32 build32)
  set_target_properties(OpenKneeboard-OpenXR-Rust32 PROPERTIES OUTPUT_FILE "${32BIT_RUST_OPENXR_DLL}")
  add_dependencies(runtime-dependencies OpenKneeboard-OpenXR-Rust32)
endif ()

configure_file(
  OpenKneeboard-OpenXR-Rust.in.json
  OpenKneeboard-OpenXR-Rust.intermediate.json
  NEWLINE_STYLE UNIX
)
file(
  GENERATE
  OUTPUT "${RUST_OPENXR_JSON}"
  INPUT "${CMAKE_CURRENT_BINARY_DIR}/OpenKneeboard-OpenXR-Rust.intermediate.json"
  NEWLINE_STYLE UNIX
)

ok_add_library(OpenKneeboard-WindowCaptureHook MODULE DUALARCH WindowCaptureHook.cpp)
target_link_libraries(
  OpenKneeboard-WindowCaptureHook
  PRIVATE
  OpenKneeboard-config
  OpenKneeboard-tracing
  detours-ext
)
set_target_properties(
  OpenKneeboard-WindowCaptureHook
  PROPERTIES

  # Needed to avoid C function name mangling on 32-bit builds
  WINDOWS_EXPORT_ALL_SYMBOLS ON
)
