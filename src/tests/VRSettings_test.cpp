// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#include <OpenKneeboard/VRSettings.hpp>
#include <OpenKneeboard/json/VRSettings.hpp>
#include <OpenKneeboard/json.hpp>

#include <gtest/gtest.h>

using namespace OpenKneeboard;

TEST(VRSettings, DefaultRoundTrip) {
  // VRSettings{} -> json -> VRSettings == default.
  const VRSettings original {};
  const nlohmann::json j = original;
  EXPECT_EQ(j.get<VRSettings>(), original) << j.dump(2);
}

TEST(VRSettings, SparseJsonKeepsDefaults) {
  // json with one changed field -> that field set, all others == default.
  const auto j = nlohmann::json::parse(R"({"EnableSteamVR": false})");
  const auto parsed = j.get<VRSettings>();

  VRSettings expected {};
  expected.mEnableSteamVR = false;
  EXPECT_EQ(parsed, expected);
}
