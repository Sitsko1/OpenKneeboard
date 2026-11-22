//! Fails when a Rust wire struct's size, alignment or field offsets differ
//! from the C++ layout manifest.

use core::mem::{align_of, offset_of, size_of};
use okb_shm::wire::*;
use serde_json::Value;

/// Checks one struct against its manifest entry. `$field` arms map C++ member
/// names to Rust offsets.
macro_rules! check {
    ($m:expr, $name:literal, $ty:ty, { $($cpp:literal => $rs:ident),* $(,)? }) => {{
        let entry = &$m["structs"][$name];
        assert_eq!(size_of::<$ty>() as u64, entry["size"].as_u64().unwrap(), concat!($name, " size"));
        assert_eq!(align_of::<$ty>() as u64, entry["align"].as_u64().unwrap(), concat!($name, " align"));
        for (field, want) in entry["fields"].as_object().unwrap() {
            let got = match field.as_str() {
                $($cpp => offset_of!($ty, $rs),)*
                other => panic!("{}.{other}: manifest field has no Rust mapping", $name),
            };
            assert_eq!(got as u64, want.as_u64().unwrap(), "{}.{field} offset", $name);
        }
    }};
}

#[test]
fn layout_matches_manifest() {
    let m: Value = serde_json::from_str(
        &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/layout/shm-v1.json"))
            .expect("manifest readable"),
    )
    .expect("manifest is JSON");
    assert_eq!(m["structs"].as_object().unwrap().len(), 13, "struct count");

    check!(m, "SharedData", SharedData, {
        "mMagic" => magic, "mGPULUID" => gpu_luid, "mFrameNumber" => frame_number,
        "mSessionID" => session_id, "mFlags" => flags, "mFeederProcessID" => feeder_process_id,
        "mPadding" => padding, "mFrames" => frames,
    });
    check!(m, "Frame", Frame, {
        "mTexture" => texture, "mFence" => fence, "mReadyReadFenceValue" => ready_read_fence_value,
        "mConfig" => config, "mLayers" => layers, "mLayerCount" => layer_count,
    });
    check!(m, "Config", Config, {
        "mGlobalInputLayerID" => global_input_layer_id, "mVR" => vr,
        "mTextureSize" => texture_size, "mTint" => tint,
    });
    check!(m, "LayerConfig", LayerConfig, {
        "mLayerID" => layer_id, "mVREnabled" => vr_enabled, "mVR" => vr,
    });
    check!(m, "VRRenderSettings", VrRenderSettings, {
        "mQuirks" => quirks, "mEnableGazeInputFocus" => enable_gaze_input_focus,
        "mForceZoom" => force_zoom, "mRecenterCount" => recenter_count,
    });
    check!(m, "Quirks", Quirks, { "mOpenXR_Upscaling" => openxr_upscaling });
    check!(m, "VRLayer", VrLayer, {
        "mPose" => pose, "mPhysicalSize" => physical_size, "mEnableGazeZoom" => enable_gaze_zoom,
        "mZoomScale" => zoom_scale, "mGazeTargetScale" => gaze_target_scale,
        "mOpacity" => opacity, "mLocationOnTexture" => location_on_texture,
    });
    check!(m, "VRPose", VrPose, {
        "mX" => x, "mEyeY" => eye_y, "mZ" => z, "mRX" => rx, "mRY" => ry, "mRZ" => rz,
    });
    check!(m, "GazeTargetScale", GazeTargetScale, { "mVertical" => vertical, "mHorizontal" => horizontal });
    check!(m, "VROpacitySettings", VrOpacitySettings, { "mNormal" => normal, "mGaze" => gaze });
    check!(m, "PixelRect", PixelRect, { "mOffset" => offset, "mSize" => size, "mOrigin" => origin });
    check!(m, "PixelSize", PixelSize, { "mWidth" => width, "mHeight" => height });
    check!(m, "ActiveConsumers", ActiveConsumers, {
        "mOpenVR" => open_vr, "mOpenXR_D3D11" => open_xr_d3d11, "mOpenXR_D3D12" => open_xr_d3d12,
        "mOpenXR_Vulkan2" => open_xr_vulkan2, "mViewer" => viewer,
        "mElevatedConsumerProcessID" => elevated_consumer_process_id,
        "mNonVRPixelSize" => non_vr_pixel_size, "mGlobalInputViewID" => global_input_view_id,
        "mActiveViewID" => active_view_id,
    });
}
