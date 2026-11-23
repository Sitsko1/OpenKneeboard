// OpenKneeboard
//
// Copyright (c) 2025 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#include <OpenKneeboard/Filesystem.hpp>
#include <OpenKneeboard/OpenXRMode.hpp>
#include <OpenKneeboard/RunSubprocessAsync.hpp>
#include <OpenKneeboard/RuntimeFiles.hpp>

#include <OpenKneeboard/dprint.hpp>

#include <format>

namespace OpenKneeboard {

static auto LaunchAndWaitForOpenXRHelperSubprocess(
  RunAs runas,
  std::wstring_view command) {
  const auto layerPath = RuntimeFiles::GetInstallationDirectory().wstring();
  const auto exePath = (Filesystem::GetRuntimeDirectory()
                        / RuntimeFiles::OPENXR_REGISTER_LAYER_HELPER)
                         .wstring();

  const auto commandLine = std::format(L"{} \"{}\"", command, layerPath);
  return RunSubprocessAsync(exePath, commandLine, runas);
}

static task<void> SetModeWithHelperProcess(
  std::wstring_view bitness,
  OpenXRMode mode,
  OpenXRLayerImplementation impl) {
  switch (mode) {
    case OpenXRMode::Disabled:
      // Disables both implementations
      co_await LaunchAndWaitForOpenXRHelperSubprocess(
        RunAs::Administrator, std::format(L"disable-HKLM-{}", bitness));
      break;
    case OpenXRMode::AllUsers:
      co_await LaunchAndWaitForOpenXRHelperSubprocess(
        RunAs::Administrator,
        std::format(
          L"enable-HKLM-{}{}",
          bitness,
          impl == OpenXRLayerImplementation::Rust ? L"-rust" : L""));
      break;
  }
}

task<void> SetOpenXR64ModeWithHelperProcess(
  OpenXRMode mode,
  OpenXRLayerImplementation impl) {
  return SetModeWithHelperProcess(L"64", mode, impl);
}

task<void> SetOpenXR32ModeWithHelperProcess(
  OpenXRMode mode,
  OpenXRLayerImplementation impl) {
  return SetModeWithHelperProcess(L"32", mode, impl);
}

}// namespace OpenKneeboard
