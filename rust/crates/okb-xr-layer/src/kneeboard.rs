//! The D3D11 session kneeboard: C++ `OpenXRKneeboard` (`xrEndFrame` and its
//! helpers) and `OpenXRD3D11Kneeboard`, line for line.
#![allow(unsafe_code)]

use std::ptr;
use std::sync::Mutex;

use okb_shm::wire::PixelSize;
use okb_shm::{ReadError, active_consumers};
use okb_vr::{ActiveViewUpdate, Pose, Quat, SpriteAtlas, Vec3, VrKneeboard};
use openxr_sys as xr;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_RENDER_TARGET_VIEW_DESC, D3D11_RENDER_TARGET_VIEW_DESC_0, D3D11_RTV_DIMENSION_TEXTURE2D,
    D3D11_TEX2D_RTV, ID3D11Device, ID3D11DeviceContext1, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,
};
use windows::core::Interface;

use crate::compose::{atlas_key, atlas_sizes, compose, is_upscaling_enabled};
use crate::d3d11::{DeviceContextState, Renderer, ShmClient, SwapchainImage};
use crate::{Next, failed, log};

const POSE_IDENTITY: xr::Posef = xr::Posef {
    orientation: xr::Quaternionf {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    },
    position: xr::Vector3f {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    },
};

/// C++ `Pose {}`.
const POSE_DEFAULT: Pose = Pose {
    position: Vec3::new(0.0, 0.0, 0.0),
    orientation: Quat {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    },
};

/// The C++ `GetHMDPose` function statics: process-wide, keyed by display time,
/// starting at 0.
static HMD_POSE_CACHE: Mutex<(i64, Pose)> = Mutex::new((0, POSE_DEFAULT));

/// C++ `check_xrresult`: `fatal()` on failure.
fn check(result: xr::Result, what: &str) {
    assert!(!failed(result), "OpenXR call failed: {what}: {result:?}");
}

/// C++ `GetXrPosef`.
fn xr_pose(pose: &Pose) -> xr::Posef {
    let (p, o) = (pose.position, pose.orientation);
    xr::Posef {
        orientation: xr::Quaternionf {
            x: o.x,
            y: o.y,
            z: o.z,
            w: o.w,
        },
        position: xr::Vector3f {
            x: p.x,
            y: p.y,
            z: p.z,
        },
    }
}

pub(crate) struct Kneeboard {
    next: Next,
    max_layer_count: u32,
    local_space: xr::Space,
    view_space: xr::Space,
    runtime_name: String,

    swapchain: xr::Swapchain,
    swapchain_size: PixelSize,
    /// C++ `mSwapchainResources` for the current swapchain.
    images: Vec<SwapchainImage>,

    atlas_key: u64,
    atlas: Option<SpriteAtlas>,

    vr: VrKneeboard,
    names: okb_shm::Names,

    device: ID3D11Device,
    ctx: ID3D11DeviceContext1,
    renderer: Renderer,
    shm: Option<ShmClient>,
    state: DeviceContextState,

    /// The quads appended to the game's layers, and the forwarded layer list;
    /// kept until the next call, as in C++.
    quads: Vec<xr::CompositionLayerQuad>,
    layers: Vec<*const xr::CompositionLayerBaseHeader>,
}

impl Kneeboard {
    /// C++ `OpenXRKneeboard` then `OpenXRD3D11Kneeboard` constructors.
    ///
    /// # Panics
    /// If a runtime call or a D3D11 call fails (C++ `fatal`/`check_hresult`).
    pub(crate) fn new(
        next: &Next,
        instance: xr::Instance,
        system: xr::SystemId,
        session: xr::Session,
        runtime_name: &str,
        binding: &xr::GraphicsBindingD3D11KHR,
    ) -> Self {
        log("OpenXRKneeboard::OpenXRKneeboard");
        // SAFETY: all-zero is a valid `XrSystemProperties` (null `next`, zero
        // ids and counts, empty name).
        let mut props: xr::SystemProperties = unsafe { std::mem::zeroed() };
        props.ty = xr::StructureType::SYSTEM_PROPERTIES;
        // SAFETY: valid out-struct.
        check(
            unsafe {
                next.get_system_properties.expect("xrGetSystemProperties")(
                    instance, system, &mut props,
                )
            },
            "xrGetSystemProperties",
        );
        let graphics = props.graphics_properties;
        log(&format!(
            "XR system: {}",
            crate::bounded(&props.system_name)
        ));
        log(&format!(
            "System supports up to {} layers, with a suggested resolution of {}x{}",
            graphics.max_layer_count,
            graphics.max_swapchain_image_width,
            graphics.max_swapchain_image_height
        ));

        let create_space = |ty| {
            let info = xr::ReferenceSpaceCreateInfo {
                ty: xr::StructureType::REFERENCE_SPACE_CREATE_INFO,
                next: ptr::null(),
                reference_space_type: ty,
                pose_in_reference_space: POSE_IDENTITY,
            };
            let mut space = xr::Space::NULL;
            // SAFETY: valid create info and out-pointer.
            check(
                unsafe {
                    next.create_reference_space.expect("xrCreateReferenceSpace")(
                        session, &info, &mut space,
                    )
                },
                "xrCreateReferenceSpace",
            );
            space
        };
        let local_space = create_space(xr::ReferenceSpaceType::LOCAL);
        let view_space = create_space(xr::ReferenceSpaceType::VIEW);

        if runtime_name.starts_with("Varjo") {
            log("Varjo runtime detected");
        }

        log("OpenXRD3D11Kneeboard::OpenXRD3D11Kneeboard");
        // SAFETY: the caller checked `binding.device` is non-null; the game
        // keeps it alive for the session, and `cloned` adds our reference.
        let device = unsafe { ID3D11Device::from_raw_borrowed(&binding.device) }
            .expect("null D3D11 device")
            .clone();
        // SAFETY: plain COM call on a live device.
        let ctx = unsafe { device.GetImmediateContext() }.expect("GetImmediateContext");
        let ctx: ID3D11DeviceContext1 = ctx.cast().expect("ID3D11DeviceContext1");
        let renderer = Renderer::new(&device);
        let shm = ShmClient::new(&device);

        Self {
            next: *next,
            max_layer_count: graphics.max_layer_count,
            local_space,
            view_space,
            runtime_name: runtime_name.to_owned(),
            swapchain: xr::Swapchain::NULL,
            swapchain_size: PixelSize::default(),
            images: Vec::new(),
            atlas_key: 0,
            atlas: None,
            vr: VrKneeboard::new(),
            names: crate::identity::shm_names(),
            device,
            ctx,
            renderer,
            shm,
            state: DeviceContextState::default(),
            quads: Vec::new(),
            layers: Vec::new(),
        }
    }

    /// C++ `OpenXRKneeboard::xrEndFrame`.
    ///
    /// # Panics
    /// Where C++ terminates: a failed swapchain call, an SHM handle
    /// duplication failure, an atlas that does not fit, an out-of-range index.
    pub(crate) fn end_frame(
        &mut self,
        session: xr::Session,
        frame_end_info: *const xr::FrameEndInfo,
    ) -> xr::Result {
        let next_end_frame = self.next.end_frame.expect("xrEndFrame");
        // SAFETY: the caller's arguments, passed through.
        let forward = || unsafe { next_end_frame(session, frame_end_info) };
        // SAFETY: the game passes a valid `XrFrameEndInfo`.
        let info = unsafe { *frame_end_info };
        if info.layer_count == 0 {
            return forward();
        }

        // C++ `if (!shm)`: no reader, or no feeder.
        let Some(shm) = self.shm.as_mut() else {
            return forward();
        };
        if !shm.reader().has_feeder() {
            return forward();
        }
        let frame = match shm.reader().maybe_get() {
            Ok(frame) => frame,
            Err(ReadError::Frame(_)) => return forward(),
            // C++ throws from `MaybeGet`, inside a `noexcept` hook.
            Err(ReadError::DuplicateHandle(e)) => panic!("DuplicateHandle failed: {e}"),
        };
        if frame.layers.is_empty() {
            return forward();
        }

        // C++ `GetSpriteAtlas`.
        let upscale = is_upscaling_enabled(&frame.config, &self.runtime_name);
        let key = atlas_key(&frame.config, upscale);
        if self.atlas_key != key {
            self.atlas_key = key;
            self.atlas = Some(
                SpriteAtlas::new(&atlas_sizes(&frame.layers, upscale))
                    .expect("Can not fit SpriteAtlas in resource limits"),
            );
        }
        let swapchain_size = self.atlas.as_ref().expect("no sprite atlas").size();

        if self.swapchain != xr::Swapchain::NULL && self.swapchain_size != swapchain_size {
            self.images.clear();
            // SAFETY: our swapchain; the result is ignored, as in C++.
            unsafe { self.next.destroy_swapchain.expect("xrDestroySwapchain")(self.swapchain) };
            self.swapchain = xr::Swapchain::NULL;
        }
        if self.swapchain == xr::Swapchain::NULL {
            self.swapchain = self
                .create_swapchain(session, swapchain_size)
                .expect("Failed to create swapchain");
            self.swapchain_size = swapchain_size;
            log(&format!(
                "Created {}x{} swapchain",
                swapchain_size.width, swapchain_size.height
            ));
        }

        let hmd = self.hmd_pose(info.display_time);
        let (vr_layers, update) = self.vr.get_layers(&frame.config, &frame.layers, hmd);
        // C++ `GetLayers` calls `SetActiveInGameViewID` itself.
        if let ActiveViewUpdate::Set(id) = update {
            active_consumers::set_active_in_game_view_id(&self.names, id);
        }
        let atlas = self.atlas.as_ref().expect("no sprite atlas");
        let Some((quads, sprites)) = compose(
            &frame.config,
            &vr_layers,
            atlas,
            self.max_layer_count,
            info.layer_count,
        ) else {
            return forward();
        };

        // SAFETY: the game passes `layer_count` valid layer pointers.
        let game_layers =
            unsafe { std::slice::from_raw_parts(info.layers, info.layer_count as usize) };
        self.quads.clear();
        self.quads
            .extend(quads.iter().map(|q| xr::CompositionLayerQuad {
                ty: xr::StructureType::COMPOSITION_LAYER_QUAD,
                next: ptr::null(),
                // The shared texture is premultiplied.
                layer_flags: xr::CompositionLayerFlags::BLEND_TEXTURE_SOURCE_ALPHA,
                space: self.local_space,
                eye_visibility: xr::EyeVisibility::BOTH,
                sub_image: xr::SwapchainSubImage {
                    swapchain: self.swapchain,
                    image_rect: xr::Rect2Di {
                        offset: xr::Offset2Di {
                            x: q.image_rect.offset.x as i32,
                            y: q.image_rect.offset.y as i32,
                        },
                        extent: xr::Extent2Di {
                            width: q.image_rect.size.width as i32,
                            height: q.image_rect.size.height as i32,
                        },
                    },
                    image_array_index: 0,
                },
                pose: xr_pose(&q.pose),
                size: xr::Extent2Df {
                    width: q.size.x,
                    height: q.size.y,
                },
            }));
        self.layers.clear();
        self.layers.extend_from_slice(game_layers);
        self.layers
            .extend(self.quads.iter().map(|q| ptr::from_ref(q).cast()));

        let mut index = u32::MAX;
        // SAFETY: our swapchain and a valid out-pointer.
        check(
            unsafe {
                self.next
                    .acquire_swapchain_image
                    .expect("xrAcquireSwapchainImage")(
                    self.swapchain, ptr::null(), &mut index
                )
            },
            "xrAcquireSwapchainImage",
        );
        let wait_info = xr::SwapchainImageWaitInfo {
            ty: xr::StructureType::SWAPCHAIN_IMAGE_WAIT_INFO,
            next: ptr::null(),
            timeout: xr::Duration::INFINITE,
        };
        // SAFETY: our swapchain and a valid wait info.
        check(
            unsafe {
                self.next
                    .wait_swapchain_image
                    .expect("xrWaitSwapchainImage")(self.swapchain, &wait_info)
            },
            "xrWaitSwapchainImage",
        );

        // C++ `OpenXRD3D11Kneeboard::RenderLayers`.
        {
            let _state = self.state.scoped(&self.ctx);
            let shm = self.shm.as_mut().expect("no SHM client");
            let mapped = shm.map(&frame);
            self.renderer.render_layers(
                &self.images[index as usize],
                self.swapchain_size,
                &mapped,
                &sprites,
            );
        }

        // SAFETY: our swapchain.
        check(
            unsafe {
                self.next
                    .release_swapchain_image
                    .expect("xrReleaseSwapchainImage")(self.swapchain, ptr::null())
            },
            "xrReleaseSwapchainImage",
        );

        let mut next_info = info;
        next_info.layers = self.layers.as_ptr();
        next_info.layer_count = self.layers.len() as u32;
        // SAFETY: `next_info` and the layers it points at live until the call returns.
        let result = unsafe { next_end_frame(session, &next_info) };
        if failed(result) {
            log(&format!("Next xrEndFrame failed: {result:?}"));
        }
        result
    }

    /// C++ `GetHMDPose`.
    fn hmd_pose(&self, display_time: xr::Time) -> Pose {
        let mut cache = HMD_POSE_CACHE.lock().unwrap();
        if display_time.as_nanos() == cache.0 {
            return cache.1;
        }
        // SAFETY: all-zero is a valid `XrSpaceLocation`.
        let mut location: xr::SpaceLocation = unsafe { std::mem::zeroed() };
        location.ty = xr::StructureType::SPACE_LOCATION;
        // SAFETY: our spaces and a valid out-struct.
        let result = unsafe {
            self.next.locate_space.expect("xrLocateSpace")(
                self.view_space,
                self.local_space,
                display_time,
                &mut location,
            )
        };
        if result != xr::Result::SUCCESS {
            return POSE_DEFAULT;
        }
        let desired =
            xr::SpaceLocationFlags::ORIENTATION_VALID | xr::SpaceLocationFlags::POSITION_VALID;
        if !location.location_flags.contains(desired) {
            return POSE_DEFAULT;
        }
        let (p, o) = (location.pose.position, location.pose.orientation);
        *cache = (
            display_time.as_nanos(),
            Pose {
                position: Vec3::new(p.x, p.y, p.z),
                orientation: Quat {
                    x: o.x,
                    y: o.y,
                    z: o.z,
                    w: o.w,
                },
            },
        );
        cache.1
    }

    /// C++ `OpenXRD3D11Kneeboard::GetDXGIFormats`: `(texture, render target view)`.
    fn dxgi_formats(&self, session: xr::Session) -> (DXGI_FORMAT, DXGI_FORMAT) {
        let enumerate = self
            .next
            .enumerate_swapchain_formats
            .expect("xrEnumerateSwapchainFormats");
        let mut count = 0;
        // SAFETY: valid out-pointer; a zero capacity takes no array.
        if failed(unsafe { enumerate(session, 0, &mut count, ptr::null_mut()) }) {
            log("Failed to get swapchain format count");
            return Default::default();
        }
        let mut formats = vec![0i64; count as usize];
        // SAFETY: `formats` holds `count` elements.
        if failed(unsafe { enumerate(session, count, &mut count, formats.as_mut_ptr()) })
            || count == 0
        {
            log("Failed to enumerate swapchain formats");
            return Default::default();
        }
        for format in &formats {
            log(&format!("Runtime supports swapchain format: {format}"));
        }
        // The shared texture is `DXGI_FORMAT_B8G8R8A8_UNORM`.
        for (texture, rtv) in [
            (DXGI_FORMAT_B8G8R8A8_UNORM_SRGB, DXGI_FORMAT_B8G8R8A8_UNORM),
            (DXGI_FORMAT_R8G8B8A8_UNORM_SRGB, DXGI_FORMAT_R8G8B8A8_UNORM),
        ] {
            if formats.contains(&i64::from(texture.0)) {
                return (texture, rtv);
            }
        }
        let first = DXGI_FORMAT(formats[0] as i32);
        (first, first)
    }

    /// C++ `OpenXRD3D11Kneeboard::CreateSwapchain`; `None` is C++ `nullptr`.
    fn create_swapchain(&mut self, session: xr::Session, size: PixelSize) -> Option<xr::Swapchain> {
        log("OpenXRD3D11Kneeboard::CreateSwapchain");
        let (texture_format, rtv_format) = self.dxgi_formats(session);
        log(&format!(
            "Creating swapchain with format {}",
            texture_format.0
        ));
        let info = xr::SwapchainCreateInfo {
            ty: xr::StructureType::SWAPCHAIN_CREATE_INFO,
            next: ptr::null(),
            create_flags: xr::SwapchainCreateFlags::EMPTY,
            usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT,
            format: i64::from(texture_format.0),
            sample_count: 1,
            width: size.width,
            height: size.height,
            face_count: 1,
            array_size: 1,
            mip_count: 1,
        };
        let mut swapchain = xr::Swapchain::NULL;
        // SAFETY: valid create info and out-pointer.
        let result = unsafe {
            self.next.create_swapchain.expect("xrCreateSwapchain")(session, &info, &mut swapchain)
        };
        if failed(result) {
            log(&format!("Failed to create swapchain: {result:?}"));
            return None;
        }

        let enumerate = self
            .next
            .enumerate_swapchain_images
            .expect("xrEnumerateSwapchainImages");
        let mut count = 0;
        // SAFETY: valid out-pointer; a zero capacity takes no array.
        let result = unsafe { enumerate(swapchain, 0, &mut count, ptr::null_mut()) };
        if count == 0 || failed(result) {
            // Quirk kept from C++: the swapchain is not destroyed here.
            log(&format!("No images in swapchain: {result:?}"));
            return None;
        }
        log(&format!("{count} images in swapchain"));

        let mut images = vec![
            xr::SwapchainImageD3D11KHR {
                ty: xr::StructureType::SWAPCHAIN_IMAGE_D3D11_KHR,
                next: ptr::null_mut(),
                texture: ptr::null_mut(),
            };
            count as usize
        ];
        // SAFETY: `images` holds `count` D3D11 image structs.
        let result = unsafe { enumerate(swapchain, count, &mut count, images.as_mut_ptr().cast()) };
        let destroy = self.next.destroy_swapchain.expect("xrDestroySwapchain");
        if failed(result) {
            log(&format!(
                "Failed to enumerate images in swapchain: {result:?}"
            ));
            // SAFETY: the swapchain created above.
            unsafe { destroy(swapchain) };
            return None;
        }
        if images[0].ty != xr::StructureType::SWAPCHAIN_IMAGE_D3D11_KHR {
            log("Swap chain is not a D3D11 swapchain");
            // SAFETY: the swapchain created above.
            unsafe { destroy(swapchain) };
            return None;
        }

        self.images = images[..count as usize]
            .iter()
            .map(|image| {
                // SAFETY: the runtime's live swapchain texture; `cloned` adds our reference.
                let texture = unsafe { ID3D11Texture2D::from_raw_borrowed(&image.texture) }
                    .expect("null swapchain texture")
                    .clone();
                let desc = D3D11_RENDER_TARGET_VIEW_DESC {
                    Format: rtv_format,
                    ViewDimension: D3D11_RTV_DIMENSION_TEXTURE2D,
                    Anonymous: D3D11_RENDER_TARGET_VIEW_DESC_0 {
                        Texture2D: D3D11_TEX2D_RTV { MipSlice: 0 },
                    },
                };
                let mut rtv = None;
                // SAFETY: valid texture, view description and out-pointer.
                unsafe {
                    self.device
                        .CreateRenderTargetView(&texture, Some(&desc), Some(&mut rtv))
                }
                .expect("CreateRenderTargetView");
                SwapchainImage {
                    texture,
                    rtv: rtv.expect("CreateRenderTargetView returned null"),
                }
            })
            .collect();
        Some(swapchain)
    }
}

impl Drop for Kneeboard {
    /// C++ `~OpenXRD3D11Kneeboard` releases the swapchain resources (derived
    /// members) before `~OpenXRKneeboard` destroys the spaces and swapchain.
    fn drop(&mut self) {
        self.images.clear();
        // SAFETY: our handles; results are ignored, as in C++.
        unsafe {
            if self.local_space != xr::Space::NULL {
                self.next.destroy_space.expect("xrDestroySpace")(self.local_space);
            }
            if self.view_space != xr::Space::NULL {
                self.next.destroy_space.expect("xrDestroySpace")(self.view_space);
            }
            if self.swapchain != xr::Swapchain::NULL {
                self.next.destroy_swapchain.expect("xrDestroySwapchain")(self.swapchain);
            }
        }
    }
}
