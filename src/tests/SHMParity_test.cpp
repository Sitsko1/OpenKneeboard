// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#include <OpenKneeboard/SHM.hpp>

#include <Windows.h>

#include <gtest/gtest.h>

#include <format>
#include <memory>

#include <okb_ffi.h>

using namespace OpenKneeboard;

static_assert(sizeof(OkbConfig) == sizeof(SHM::Config));
static_assert(alignof(OkbConfig) == alignof(SHM::Config));
static_assert(sizeof(OkbLayerConfig) == sizeof(SHM::LayerConfig));
static_assert(alignof(OkbLayerConfig) == alignof(SHM::LayerConfig));

namespace {

struct NamesDeleter {
  void operator()(OkbShmNames* p) const { okb_shm_names_free(p); }
};
struct WriterDeleter {
  void operator()(OkbShmWriter* p) const { okb_shm_writer_free(p); }
};
struct ReaderDeleter {
  void operator()(OkbShmReader* p) const { okb_shm_reader_free(p); }
};

// Test-only reverse domain, so this never touches a running OpenKneeboard.
std::unique_ptr<OkbShmNames, NamesDeleter> TestNames() {
  const auto domain = std::format("okb-test.ffi.{}", GetCurrentProcessId());
  return std::unique_ptr<OkbShmNames, NamesDeleter> {
    okb_shm_names_new(domain.c_str(), 1, 0, 0, 0)};
}

constexpr uint64_t Gpu = 1;
constexpr uint32_t ViewerKind = 0xFFFFFFFF;// SHM::ConsumerKind::Viewer

}// namespace

TEST(SHMFfi, WriterReaderRoundTrip) {
  const auto names = TestNames();
  ASSERT_NE(names, nullptr);

  std::unique_ptr<OkbShmWriter, WriterDeleter> writer {
    okb_shm_writer_new(names.get(), Gpu)};
  ASSERT_NE(writer, nullptr);

  int sessionChanges = 0;
  std::unique_ptr<OkbShmReader, ReaderDeleter> reader {okb_shm_reader_new(
    names.get(),
    ViewerKind,
    Gpu,
    [](void* ctx) { ++*static_cast<int*>(ctx); },
    &sessionChanges)};
  ASSERT_NE(reader, nullptr);
  EXPECT_FALSE(okb_shm_reader_has_feeder(reader.get()));

  // Zero-layer frame: sets the feeder-attached flag, but has no layers.
  const SHM::Config config {};
  okb_shm_writer_lock(writer.get());
  const auto info = okb_shm_writer_begin_frame(writer.get());
  okb_shm_writer_submit_frame(
    writer.get(),
    info,
    reinterpret_cast<const OkbConfig*>(&config),
    nullptr,
    0,
    0,
    0);
  EXPECT_EQ(okb_shm_writer_frame_count(writer.get()), 1);
  okb_shm_writer_unlock(writer.get());
  EXPECT_EQ(okb_shm_writer_frame_count(writer.get()), 1);// unlocked, as C++

  EXPECT_TRUE(okb_shm_reader_has_feeder(reader.get()));
  OkbFrame frame {};
  EXPECT_EQ(okb_shm_reader_maybe_get(reader.get(), &frame), 3);// NoLayers
  EXPECT_EQ(sessionChanges, 1);

  EXPECT_TRUE(okb_shm_writer_try_lock(writer.get()));
  okb_shm_writer_detach(writer.get());
  okb_shm_writer_unlock(writer.get());
  EXPECT_FALSE(okb_shm_reader_has_feeder(reader.get()));
  EXPECT_EQ(okb_shm_reader_maybe_get(reader.get(), &frame), 2);// NoFeeder
}
