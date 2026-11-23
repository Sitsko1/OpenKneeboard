// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.
#include <OpenKneeboard/SHM.hpp>

#include <gtest/gtest.h>

#include <chrono>
#include <future>
#include <mutex>

using namespace OpenKneeboard;
using namespace std::chrono_literals;

namespace {

class TestReader final : public SHM::Reader {
 public:
  using SHM::Reader::Reader;
  int mSessionChanges {0};

 protected:
  void OnSessionChanged() override { ++mSessionChanges; }
};

constexpr uint64_t GpuA = 1;
constexpr uint64_t GpuB = 2;

// The SHM name includes the full version, including the build number (0 for
// local builds), so only a locally built OpenKneeboard collides with these
// tests.
class SHMTest : public ::testing::Test {
 protected:
  void SetUp() override {
    TestReader probe(SHM::ConsumerKind::Viewer, GpuA);
    if (static_cast<bool>(probe)) {
      GTEST_SKIP()
        << "OpenKneeboard appears to be running; close it to run SHM tests";
    }
  }
};

// Submits a frame with no layers and no handles. Unlike SubmitEmptyFrame(),
// this sets the feeder-attached flag.
void SubmitZeroLayerFrame(SHM::Writer& writer) {
  std::unique_lock lock(writer);
  const auto info = writer.BeginFrame();
  writer.SubmitFrame(info, {}, {}, nullptr, nullptr);
}

void SubmitEmptyFrame(SHM::Writer& writer) {
  std::unique_lock lock(writer);
  writer.SubmitEmptyFrame();
}

}// namespace

TEST_F(SHMTest, NoWriterMeansNoFeeder) {
  // Reader only -> MaybeGet().error() == NoFeeder; bool(reader) == false.
  TestReader reader(SHM::ConsumerKind::Viewer, GpuA);
  EXPECT_FALSE(static_cast<bool>(reader));
  const auto frame = reader.MaybeGet();
  ASSERT_FALSE(frame.has_value());
  EXPECT_EQ(frame.error(), SHM::Frame::Error::NoFeeder);
}

TEST_F(SHMTest, EmptyFrameCharacterization) {
  // Writer(luid 1) + SubmitEmptyFrame(); Reader(Viewer, luid 1).MaybeGet().
  // Observed: NoFeeder, not NoLayers - SubmitEmptyFrame() never sets the
  // feeder-attached flag, so a writer that has only submitted empty frames
  // is indistinguishable from no writer.
  SHM::Writer writer(GpuA);
  ASSERT_TRUE(static_cast<bool>(writer));
  SubmitEmptyFrame(writer);

  TestReader reader(SHM::ConsumerKind::Viewer, GpuA);
  const auto frame = reader.MaybeGet();
  ASSERT_FALSE(frame.has_value());
  EXPECT_EQ(frame.error(), SHM::Frame::Error::NoFeeder);
}

TEST_F(SHMTest, GpuMismatchCharacterization) {
  // Writer(luid 1) submits a zero-layer frame (feeder flag set);
  // Reader(Viewer, luid 2).MaybeGet() -> IncorrectGPU. The same reader on
  // luid 1 gets NoLayers: the GPU check comes before the layer-count check.
  SHM::Writer writer(GpuA);
  ASSERT_TRUE(static_cast<bool>(writer));
  SubmitZeroLayerFrame(writer);

  TestReader wrongGpu(SHM::ConsumerKind::Viewer, GpuB);
  const auto mismatch = wrongGpu.MaybeGet();
  ASSERT_FALSE(mismatch.has_value());
  EXPECT_EQ(mismatch.error(), SHM::Frame::Error::IncorrectGPU);

  TestReader rightGpu(SHM::ConsumerKind::Viewer, GpuA);
  const auto match = rightGpu.MaybeGet();
  ASSERT_FALSE(match.has_value());
  EXPECT_EQ(match.error(), SHM::Frame::Error::NoLayers);
}

TEST_F(SHMTest, DetachedWriterMeansNoFeeder) {
  // Writer submits, then Detach() -> reader sees NoFeeder.
  SHM::Writer writer(GpuA);
  ASSERT_TRUE(static_cast<bool>(writer));
  SubmitZeroLayerFrame(writer);

  TestReader reader(SHM::ConsumerKind::Viewer, GpuA);
  ASSERT_TRUE(static_cast<bool>(reader));
  {
    std::unique_lock lock(writer);
    writer.Detach();
  }
  EXPECT_FALSE(static_cast<bool>(reader));
  const auto frame = reader.MaybeGet();
  ASSERT_FALSE(frame.has_value());
  EXPECT_EQ(frame.error(), SHM::Frame::Error::NoFeeder);
}

TEST_F(SHMTest, ReaderBlocksWhileWriterHoldsLock) {
  // Writer.lock() on this thread; reader MaybeGet() on another thread does
  // not return within 200 ms, and returns after unlock(). Pins spec
  // Section 2.2 item 1 (the INFINITE wait) as current behavior; Phase 2
  // inverts this test.
  SHM::Writer writer(GpuA);
  ASSERT_TRUE(static_cast<bool>(writer));
  SubmitZeroLayerFrame(writer);
  TestReader reader(SHM::ConsumerKind::Viewer, GpuA);

  writer.lock();
  auto pending = std::async(std::launch::async, [&] {
    return reader.MaybeGet();
  });
  const auto status = pending.wait_for(200ms);
  writer.unlock();
  EXPECT_EQ(status, std::future_status::timeout);

  ASSERT_EQ(pending.wait_for(5s), std::future_status::ready);
  const auto frame = pending.get();
  ASSERT_FALSE(frame.has_value());
  EXPECT_EQ(frame.error(), SHM::Frame::Error::NoLayers);
}
