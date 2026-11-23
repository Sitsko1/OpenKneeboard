// OpenKneeboard
//
// Copyright (c) 2026 Fred Emmott <fred@fredemmott.com>
//
// This program is open source; see the LICENSE file in the root of the
// OpenKneeboard repository.

// Every OpenKneeboard executable defines gTraceProvider. The tests never
// register it, so trace writes from library code are no-ops.
#include <OpenKneeboard/tracing.hpp>

namespace OpenKneeboard {
/* PS>
 * [System.Diagnostics.Tracing.EventSource]::new("OpenKneeboard.Tests")
 * 46b582af-4d20-53ae-cd11-9edb829ce073
 */
TRACELOGGING_DEFINE_PROVIDER(
  gTraceProvider,
  "OpenKneeboard.Tests",
  (0x46b582af, 0x4d20, 0x53ae, 0xcd, 0x11, 0x9e, 0xdb, 0x82, 0x9c, 0xe0, 0x73));
}// namespace OpenKneeboard
