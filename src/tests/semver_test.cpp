// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#include <OpenKneeboard/semver.hpp>

#include <gtest/gtest.h>

using namespace OpenKneeboard;

TEST(SemVer, CompareOrdersByPrecedence) {
  EXPECT_EQ(CompareSemVer("1.2.3", "1.2.4"), ThreeWayCompareResult::LessThan);
  EXPECT_EQ(
    CompareSemVer("1.10.0", "1.9.0"), ThreeWayCompareResult::GreaterThan);
  EXPECT_EQ(CompareSemVer("1.2.3", "1.2.3"), ThreeWayCompareResult::Equal);
  EXPECT_EQ(
    CompareSemVer("1.2.3-beta", "1.2.3"), ThreeWayCompareResult::LessThan);
}

TEST(SemVer, CompareVersionsAcceptsShortForms) {
  // The plugin docs say OKB versions may have 1-4 components.
  EXPECT_EQ(CompareVersions("1.9", "1.9.0"), ThreeWayCompareResult::Equal);
  EXPECT_EQ(
    CompareVersions("1.13.0", "1.9"), ThreeWayCompareResult::GreaterThan);
}

TEST(SemVer, ToSemVerStringCharacterization) {
  // Characterization: record what the current code returns.
  // Observed: 4-component versions pass through unchanged, so the result is
  // not valid SemVer (CompareSemVer() then logs and returns Equal).
  EXPECT_EQ(ToSemVerString("1.2.3.4"), /* observed */ "1.2.3.4");
}
