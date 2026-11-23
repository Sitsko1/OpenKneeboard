// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#include <OpenKneeboard/OpenXRLayerRegistry.hpp>

#include <algorithm>

namespace OpenKneeboard {

namespace {
bool EndsWithAny(
  std::wstring_view path,
  std::span<const std::wstring_view> names) {
  return std::ranges::any_of(
    names, [&](auto name) { return path.ends_with(name); });
}
}// namespace

std::vector<std::wstring> LayersToDisableOnEnable(
  std::span<const std::wstring> registeredPaths,
  std::wstring_view chosenJsonPath,
  std::span<const std::wstring_view> okbJsonFileNames) {
  std::vector<std::wstring> ret;
  for (const auto& path: registeredPaths) {
    if (path != chosenJsonPath && EndsWithAny(path, okbJsonFileNames)) {
      ret.push_back(path);
    }
  }
  return ret;
}

std::vector<std::wstring> LayersToDisable(
  std::span<const std::wstring> registeredPaths,
  std::span<const std::wstring_view> bitnessJsonFileNames) {
  std::vector<std::wstring> ret;
  for (const auto& path: registeredPaths) {
    if (EndsWithAny(path, bitnessJsonFileNames)) {
      ret.push_back(path);
    }
  }
  return ret;
}

}// namespace OpenKneeboard
