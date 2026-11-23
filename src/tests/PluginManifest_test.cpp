// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#include <OpenKneeboard/Plugin.hpp>
#include <OpenKneeboard/json.hpp>

#include <gtest/gtest.h>

#include <filesystem>
#include <fstream>

using namespace OpenKneeboard;
using TabType = Plugin::TabType;

namespace {

nlohmann::json LoadJSON(std::string_view name) {
  const auto path = std::filesystem::path {OKB_TEST_DATA_DIR} / name;
  std::ifstream f(path);
  EXPECT_TRUE(f.is_open()) << path;
  return nlohmann::json::parse(f);
}

Plugin LoadPlugin(std::string_view name) {
  return LoadJSON(name).get<Plugin>();
}

}// namespace

TEST(PluginManifest, ParsesFullV1) {
  // plugin-v1-full.json -> mID, Metadata fields, one WebBrowser TabType with
  // mURI and mInitialSize as in the file.
  const auto plugin = LoadPlugin("plugin-v1-full.json");
  EXPECT_EQ(plugin.mID, "testplugin.openkneeboard-tests.invalid");

  const auto& meta = plugin.mMetadata;
  EXPECT_EQ(meta.mPluginName, "Test Plugin");
  EXPECT_EQ(meta.mPluginReadableVersion, "v1.2 (test)");
  EXPECT_EQ(meta.mPluginSemanticVersion, "1.2.0");
  EXPECT_EQ(meta.mOKBMinimumVersion, "1.10");
  EXPECT_EQ(meta.mOKBMaximumTestedVersion, "1.13.0");
  EXPECT_EQ(meta.mAuthor, "OpenKneeboard tests");
  EXPECT_EQ(meta.mWebsite, "https://openkneeboard-tests.invalid/");

  ASSERT_EQ(plugin.mTabTypes.size(), 1);
  const auto& tab = plugin.mTabTypes.front();
  EXPECT_EQ(tab.mID, "testplugin.openkneeboard-tests.invalid;dashboard");
  EXPECT_EQ(tab.mName, "Test Dashboard");
  EXPECT_EQ(tab.mGlyph, "\xEE\xA0\x8F");// U+E80F as UTF-8
  EXPECT_EQ(tab.mImplementation, TabType::Implementation::WebBrowser);
  ASSERT_EQ(tab.mCustomActions.size(), 1);
  EXPECT_EQ(
    tab.mCustomActions.front().mID,
    "testplugin.openkneeboard-tests.invalid;dashboard;reset");
  EXPECT_EQ(tab.mCustomActions.front().mName, "Reset");

  const auto args = std::get_if<TabType::WebBrowserArgs>(&tab.mImplementationArgs);
  ASSERT_NE(args, nullptr);
  EXPECT_EQ(args->mURI, "plugin://index.html");
  EXPECT_EQ(args->mInitialSize, (PixelSize {800, 1200}));
}

TEST(PluginManifest, AppliesDefaults) {
  // plugin-v1-minimal.json -> mOKBMinimumVersion == "1.9",
  // WebBrowserArgs::mInitialSize == {1024, 768}.
  const auto plugin = LoadPlugin("plugin-v1-minimal.json");
  EXPECT_EQ(plugin.mMetadata.mOKBMinimumVersion, "1.9");
  ASSERT_EQ(plugin.mTabTypes.size(), 1);
  const auto args = std::get_if<TabType::WebBrowserArgs>(
    &plugin.mTabTypes.front().mImplementationArgs);
  ASSERT_NE(args, nullptr);
  EXPECT_EQ(args->mInitialSize, (PixelSize {1024, 768}));
}

TEST(PluginManifest, UnknownImplementationIsPreserved) {
  // TabType with Implementation "SomethingNew" -> Implementation::Invalid with
  // InvalidImplementationArgs{mName == "SomethingNew"}.
  auto j = LoadJSON("plugin-v1-full.json");
  j["TabTypes"][0]["Implementation"] = "SomethingNew";
  j["TabTypes"][0]["ImplementationArgs"] = {{"Foo", 123}};
  const auto plugin = j.get<Plugin>();

  ASSERT_EQ(plugin.mTabTypes.size(), 1);
  const auto& tab = plugin.mTabTypes.front();
  EXPECT_EQ(tab.mImplementation, TabType::Implementation::Invalid);
  const auto args =
    std::get_if<TabType::InvalidImplementationArgs>(&tab.mImplementationArgs);
  ASSERT_NE(args, nullptr);
  EXPECT_EQ(args->mName, "SomethingNew");
  EXPECT_EQ(args->mArgs, (nlohmann::json {{"Foo", 123}}));
}

TEST(PluginManifest, RoundTripsUnchanged) {
  // parse(full) -> to_json -> parse == original (operator==).
  const auto original = LoadPlugin("plugin-v1-full.json");
  const nlohmann::json j = original;
  const auto reparsed = j.get<Plugin>();
  EXPECT_EQ(reparsed, original) << j.dump(2);
}
