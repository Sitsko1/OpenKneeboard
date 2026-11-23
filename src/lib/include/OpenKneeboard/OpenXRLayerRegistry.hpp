// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#pragma once

#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace OpenKneeboard {

/// Registry values to set to 1 (disabled) when enabling `chosenJsonPath`:
/// every other registered path ending in any OpenKneeboard layer JSON name
/// (the other implementation, and stale copies of the same JSON).
std::vector<std::wstring> LayersToDisableOnEnable(
  std::span<const std::wstring> registeredPaths,
  std::wstring_view chosenJsonPath,
  std::span<const std::wstring_view> okbJsonFileNames);

/// Registry values to set to 1 when disabling OpenXR for one bitness: every
/// registered path ending in either implementation's JSON name.
std::vector<std::wstring> LayersToDisable(
  std::span<const std::wstring> registeredPaths,
  std::span<const std::wstring_view> bitnessJsonFileNames);

}// namespace OpenKneeboard
