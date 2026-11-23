find_package(GTest CONFIG REQUIRED)
include(GoogleTest)

add_executable(
  OpenKneeboard-Tests
  TraceProvider.cpp
  semver_test.cpp
  RayIntersectsRect_test.cpp
  VRKneeboard_test.cpp
  VRKneeboardParity_test.cpp
  SpriteAtlasParity_test.cpp
  SHM_test.cpp
  SHMLayout_test.cpp
  SHMParity_test.cpp
  OpenXRLayerLoad_test.cpp
  PluginManifest_test.cpp
  VRSettings_test.cpp
  # ponytail: compiled directly rather than linking OpenKneeboard-App-Common,
  # which drags in the Windows App SDK, CEF, and more.
  ../app/app-common/Plugin.cpp
  ../app/app-common/VRSettings.cpp
)
target_include_directories(
  OpenKneeboard-Tests
  PRIVATE
  ../app/app-common/include
  ../../rust/crates/okb-ffi/include
)
target_compile_definitions(
  OpenKneeboard-Tests
  PRIVATE
  "OKB_TEST_DATA_DIR=\"${CMAKE_CURRENT_SOURCE_DIR}/data\""
  "OKB_RUST_DIR=\"${CMAKE_SOURCE_DIR}/rust\""
  "OKB_TEST_CPP_LAYER_DLL=\"$<TARGET_FILE:OpenKneeboard-OpenXR>\""
  "OKB_TEST_RUST_LAYER_DLL=\"$<GENEX_EVAL:$<TARGET_PROPERTY:OpenKneeboard-OpenXR-Rust,OUTPUT_FILE>>\""
)
add_dependencies(OpenKneeboard-Tests OpenKneeboard-OpenXR OpenKneeboard-OpenXR-Rust)
target_link_libraries(
  OpenKneeboard-Tests
  PRIVATE
  GTest::gtest
  GTest::gtest_main
  OpenKneeboard-Geometry2D
  OpenKneeboard-RayIntersectsRect
  OpenKneeboard-SHM
  OpenKneeboard-SpriteAtlas
  OpenKneeboard-VRKneeboard
  OpenKneeboard-config
  OpenKneeboard-json
  OpenKneeboard-semver
  OpenKneeboard-version
  okb_ffi
  ThirdParty::OpenXR
  ThirdParty::CppWinRT
  ThirdParty::JSON
)
# DISCOVERY_MODE PRE_TEST: discovery runs at ctest time, so a build never
# fails because the freshly linked test binary can't start.
# RESOURCE_LOCK: tests sharing the process-global ActiveConsumers SHM must not run in
# parallel under ctest -j.
gtest_discover_tests(
  OpenKneeboard-Tests
  DISCOVERY_MODE PRE_TEST
  PROPERTIES RESOURCE_LOCK ActiveConsumers
)
