find_package(GTest CONFIG REQUIRED)
include(GoogleTest)

add_executable(
  OpenKneeboard-Tests
  TraceProvider.cpp
  semver_test.cpp
  RayIntersectsRect_test.cpp
)
target_link_libraries(
  OpenKneeboard-Tests
  PRIVATE
  GTest::gtest
  GTest::gtest_main
  OpenKneeboard-RayIntersectsRect
  OpenKneeboard-semver
)
# DISCOVERY_MODE PRE_TEST: discovery runs at ctest time, so a build never
# fails because the freshly linked test binary can't start.
gtest_discover_tests(OpenKneeboard-Tests DISCOVERY_MODE PRE_TEST)
