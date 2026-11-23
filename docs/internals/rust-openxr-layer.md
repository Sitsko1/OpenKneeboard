---
title: Rust OpenXR layer
parent: Internals
---

# Rust OpenXR layer (Phase 1b)

OpenKneeboard ships two OpenXR API layers: the C++ layer (the default) and a Rust port. They are behavior-identical by design; see [the parity contract](rust-port-parity-contract.md). Only one is registered and enabled per bitness at a time.

## Scope

The Rust layer supports **D3D11 only** until Phases 1c (D3D12) and 1d (Vulkan). D3D12 and Vulkan sessions pass through with no kneeboard, so use the C++ layer for those games.

## Selecting the Rust layer

1. Close OpenKneeboard.
2. In `%LOCALAPPDATA%\OpenKneeboard\Settings\App.json`, add `"OpenXRLayer": "Rust"` (valid values are `"Cpp"` and `"Rust"`; a missing or unknown value means `Cpp`).
3. Start OpenKneeboard, open the VR settings page, and turn OpenXR off, then on again, for 64-bit (and 32-bit if needed). Accept the administrator prompt each time.

Enabling one implementation disables the other (and any stale registrations of the same JSON); turning OpenXR off disables both. To go back, set `"OpenXRLayer": "Cpp"` (or remove the field) and toggle OpenXR off and on again.

An install or upgrade re-registers the C++ layer for both bitnesses and disables the Rust layer, so re-select Rust afterward (toggle OpenXR off, then on, as in step 3).

## Manual hardware checklist

Run on a machine with a Quest 3 and Virtual Desktop (VDXR is the active OpenXR runtime). Run the whole list once with the C++ layer, then again with the Rust layer, and compare. Tick a box only if the stated observation is true.

Setup:

- [ ] VR settings shows OpenXR enabled for 64-bit, and `regedit` under `HKLM\SOFTWARE\Khronos\OpenXR\1\ApiLayers\Implicit` shows exactly one OpenKneeboard JSON with value `0`: `OpenKneeboard-OpenXR.json` (C++ run) or `OpenKneeboard-OpenXR-Rust.json` (Rust run); the other has value `1` or is absent.

`hello_xr -g D3D11` on VDXR:

- [ ] hello_xr starts and renders in the headset with no error dialog and no crash.
- [ ] The kneeboard is visible, at the same position and the same size as in the C++ run.
- [ ] Looking at the kneeboard makes it zoom (gaze zoom); looking away returns it to normal size.
- [ ] Recentering (the OpenKneeboard recenter action) moves the kneeboard in front of you.
- [ ] With a second view set as the global input layer, both views appear, and the global-input view is in the same place as in the C++ run.
- [ ] Closing OpenKneeboard mid-session makes the kneeboard disappear while hello_xr keeps running with no crash. Restarting OpenKneeboard brings it back.
- [ ] Exiting hello_xr leaves no crash report or orphaned process.

One 64-bit D3D11 game:

- [ ] The game launches in VR, the kneeboard is visible and usable (page turns, gaze zoom), and the game does not crash during 10 minutes of play.

Result:

- [ ] Every Rust-run observation matches the C++ run. Record any difference, with a screenshot, before ticking.
