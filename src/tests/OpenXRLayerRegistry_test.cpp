// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#include <OpenKneeboard/AppSettings.hpp>
#include <OpenKneeboard/OpenXRLayerRegistry.hpp>
#include <OpenKneeboard/json.hpp>

#include <gtest/gtest.h>

#include <array>

using namespace OpenKneeboard;
using namespace std::string_view_literals;

namespace {
using V = std::vector<std::wstring>;
constexpr std::array AllNames {
  L"OpenKneeboard-OpenXR.json"sv,
  L"OpenKneeboard-OpenXR32.json"sv,
  L"OpenKneeboard-OpenXR-Rust.json"sv,
  L"OpenKneeboard-OpenXR-Rust32.json"sv,
};
constexpr std::array Names64 {
  L"OpenKneeboard-OpenXR.json"sv,
  L"OpenKneeboard-OpenXR-Rust.json"sv,
};
const std::wstring Cpp = L"C:/okb/OpenKneeboard-OpenXR.json";
const std::wstring Rust = L"C:/okb/OpenKneeboard-OpenXR-Rust.json";
const std::wstring Other = L"C:/other/Thing.json";
}// namespace

TEST(OpenXRLayerRegistry, EnablingRustDisablesCpp) {
  const V reg {Cpp, Rust};
  EXPECT_EQ(LayersToDisableOnEnable(reg, Rust, AllNames), V {Cpp});
}

TEST(OpenXRLayerRegistry, EnablingCppDisablesRust) {
  const V reg {Cpp, Rust};
  EXPECT_EQ(LayersToDisableOnEnable(reg, Cpp, AllNames), V {Rust});
}

TEST(OpenXRLayerRegistry, EnablingKeepsUnrelatedLayers) {
  const V reg {Cpp, Other};
  EXPECT_TRUE(LayersToDisableOnEnable(reg, Rust, AllNames) == V {Cpp});
}

TEST(OpenXRLayerRegistry, EnablingDisablesStaleCopiesOfSameJson) {
  const std::wstring stale = L"D:/old/OpenKneeboard-OpenXR.json";
  const V reg {Cpp, stale};
  EXPECT_EQ(LayersToDisableOnEnable(reg, Cpp, AllNames), V {stale});
}

TEST(OpenXRLayerRegistry, DisableCoversBothImplementations) {
  const std::wstring rust32 = L"C:/okb/OpenKneeboard-OpenXR-Rust32.json";
  const V reg {Cpp, Rust, Other, rust32};
  EXPECT_EQ(LayersToDisable(reg, Names64), (V {Cpp, Rust}));
}

TEST(AppSettings, OpenXRLayerRoundTrips) {
  EXPECT_EQ(
    nlohmann::json::parse("{}").get<AppSettings>().mOpenXRLayer,
    OpenXRLayerImplementation::Cpp);
  EXPECT_EQ(
    nlohmann::json::parse(R"({"OpenXRLayer":"Bogus"})")
      .get<AppSettings>()
      .mOpenXRLayer,
    OpenXRLayerImplementation::Cpp);

  AppSettings s {};
  s.mOpenXRLayer = OpenXRLayerImplementation::Rust;
  const nlohmann::json j = s;
  EXPECT_EQ(j.at("OpenXRLayer"), "Rust");
  EXPECT_EQ(j.get<AppSettings>().mOpenXRLayer, OpenXRLayerImplementation::Rust);
}
