//! OpenKneeboard OpenXR API layer: loader negotiation, `xrGetInstanceProcAddr`
//! hooks and instance/session lifecycle, ported from
//! `src/injectables/OpenXRKneeboard.cpp`.
//!
//! The FFI items here carry their own `#[allow(unsafe_code)]`; modules that
//! must not use `unsafe` opt out with `#![forbid(unsafe_code)]`.

/// Casts a hook or fake to `PFN_xrVoidFunction`, as `xrGetInstanceProcAddr`
/// returns it.
macro_rules! void_fn {
    ($f:expr, $ty:ty) => {
        // SAFETY: function pointer to function pointer of the same ABI; the
        // caller casts it back to `$ty` before calling it.
        Some(unsafe { std::mem::transmute::<$ty, pfn::VoidFunction>($f) })
    };
}

pub mod compose;
pub mod d3d11;
pub mod identity;
mod next;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

use std::ffi::{CStr, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Mutex;

use openxr_sys as xr;
use windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use xr::pfn;

use identity::{LAYER_DESCRIPTION, LAYER_IMPL_VERSION, LAYER_NAME};
pub(crate) use next::Next;

/// `XR_CURRENT_API_VERSION` of the OpenXR SDK the C++ layer is built with
/// (release-1.0.25, `third-party/openxr.cmake`), not openxr-sys's newer one.
const XR_CURRENT_API_VERSION: xr::Version = xr::Version::new(1, 0, 25);
const XR_CURRENT_LOADER_API_LAYER_VERSION: u32 = 1;

const _: () = assert!(LAYER_NAME.len() <= xr::MAX_API_LAYER_NAME_SIZE);
const _: () = assert!(LAYER_DESCRIPTION.len() <= xr::MAX_API_LAYER_DESCRIPTION_SIZE);

/// Placeholder until the D3D11 kneeboard exists: records the binding of the
/// session that gets the kneeboard. `xrEndFrame` always forwards.
#[allow(dead_code, reason = "read by the D3D11 kneeboard, which replaces this")]
pub(crate) struct Kneeboard {
    /// `XrGraphicsBindingD3D11KHR::device`, as an address.
    pub(crate) device: usize,
}

pub(crate) struct LayerState {
    pub(crate) next: Next,
    pub(crate) runtime_name: String,
    pub(crate) kneeboard: Option<Kneeboard>,
}

pub(crate) static STATE: Mutex<Option<LayerState>> = Mutex::new(None);

#[allow(unsafe_code)]
fn log(message: &str) {
    let wide: Vec<u16> = format!("OpenKneeboard-OpenXR-Rust: {message}\n")
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: NUL-terminated UTF-16 string that outlives the call.
    unsafe { OutputDebugStringW(wide.as_ptr()) };
}

/// Runs an entry point's body. A panic terminates the process, like the C++
/// layer's `fatal()`; it never unwinds into the loader.
fn guard<R>(body: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|_| std::process::abort())
}

/// C++ `Next()`; called before `xrCreateApiLayerInstance` C++ dereferences
/// null, so this panics (and so aborts).
fn next() -> Next {
    STATE
        .lock()
        .unwrap()
        .as_ref()
        .expect("OpenXR call before xrCreateApiLayerInstance")
        .next
}

fn failed(result: xr::Result) -> bool {
    result.into_raw() < 0
}

/// A fixed-size, NUL-padded OpenXR string.
fn bounded(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn copy_str(dst: &mut [c_char], src: &str) {
    for (d, s) in dst.iter_mut().zip(src.bytes()) {
        *d = s as c_char;
    }
}

#[allow(unsafe_code)]
pub(crate) unsafe extern "system" fn xr_create_session(
    instance: xr::Instance,
    create_info: *const xr::SessionCreateInfo,
    session: *mut xr::Session,
) -> xr::Result {
    guard(|| {
        let next = next();
        let mut props = xr::InstanceProperties {
            ty: xr::StructureType::INSTANCE_PROPERTIES,
            next: ptr::null_mut(),
            runtime_version: xr::Version::new(0, 0, 0),
            runtime_name: [0; xr::MAX_RUNTIME_NAME_SIZE],
        };
        // SAFETY: valid out-struct. The result is ignored, as in C++.
        unsafe {
            next.get_instance_properties
                .expect("xrGetInstanceProperties")(instance, &mut props)
        };
        let runtime_name = bounded(&props.runtime_name);
        log(&format!(
            "In xrCreateSession with OpenXR runtime: '{runtime_name}' v{}",
            props.runtime_version
        ));
        STATE
            .lock()
            .unwrap()
            .as_mut()
            .expect("no instance")
            .runtime_name = runtime_name;

        // SAFETY: the caller's arguments, passed through.
        let ret = unsafe {
            next.create_session.expect("xrCreateSession")(instance, create_info, session)
        };
        if failed(ret) {
            log(&format!("next xrCreateSession failed: {ret:?}"));
            return ret;
        }
        log("Next xrCreateSession succeeded");

        let mut state = STATE.lock().unwrap();
        let state = state.as_mut().expect("no instance");
        if state.kneeboard.is_some() {
            log("Already have a kneeboard, refusing to initialize twice");
            return xr::Result::ERROR_LIMIT_REACHED;
        }

        // SAFETY: the caller passes a valid `XrSessionCreateInfo` whose `next`
        // chain is made of valid `XrBaseInStructure`-prefixed structs.
        unsafe {
            let mut p = (*create_info).next.cast::<xr::BaseInStructure>();
            while !p.is_null() && (*p).ty != xr::StructureType::GRAPHICS_BINDING_D3D11_KHR {
                p = (*p).next;
            }
            if !p.is_null() {
                let device = (*p.cast::<xr::GraphicsBindingD3D11KHR>()).device;
                if !device.is_null() {
                    // ponytail: placeholder; the D3D11 kneeboard is built here later.
                    state.kneeboard = Some(Kneeboard {
                        device: device as usize,
                    });
                    return ret;
                }
            }

            log("Unsupported graphics API (this layer supports D3D11 only)");
            let mut p = create_info.cast::<xr::BaseInStructure>();
            while !p.is_null() {
                log(&format!("xrCreateSession next chain: {:?}", (*p).ty));
                p = (*p).next;
            }
        }
        ret
    })
}

#[allow(unsafe_code)]
pub(crate) unsafe extern "system" fn xr_destroy_session(session: xr::Session) -> xr::Result {
    guard(|| {
        let next = {
            let mut state = STATE.lock().unwrap();
            let state = state.as_mut().expect("no instance");
            state.kneeboard = None;
            state.next
        };
        // SAFETY: the caller's handle, passed through.
        unsafe { next.destroy_session.expect("xrDestroySession")(session) }
    })
}

#[allow(unsafe_code)]
pub(crate) unsafe extern "system" fn xr_destroy_instance(instance: xr::Instance) -> xr::Result {
    guard(|| {
        let next = {
            let mut state = STATE.lock().unwrap();
            let state = state.as_mut().expect("no instance");
            state.kneeboard = None;
            state.next
        };
        // SAFETY: the caller's handle, passed through.
        unsafe { next.destroy_instance.expect("xrDestroyInstance")(instance) }
    })
}

#[allow(unsafe_code)]
pub(crate) unsafe extern "system" fn xr_end_frame(
    session: xr::Session,
    frame_end_info: *const xr::FrameEndInfo,
) -> xr::Result {
    // SAFETY: the caller's arguments, passed through.
    guard(|| unsafe { next().end_frame.expect("xrEndFrame")(session, frame_end_info) })
}

#[allow(unsafe_code)]
pub(crate) unsafe extern "system" fn xr_enumerate_api_layer_properties(
    property_capacity_input: u32,
    property_count_output: *mut u32,
    properties: *mut xr::ApiLayerProperties,
) -> xr::Result {
    guard(|| {
        // We only return our own properties, per the loader spec.
        // SAFETY: the caller passes a valid count pointer.
        unsafe { *property_count_output = 1 };
        if property_capacity_input == 0 {
            // Not XR_ERROR_SIZE_INSUFFICIENT for 0, per the spec.
            return xr::Result::SUCCESS;
        }
        // SAFETY: capacity >= 1, so the caller passes at least one element.
        let props = unsafe { &mut *properties };
        if props.ty != xr::StructureType::API_LAYER_PROPERTIES {
            return xr::Result::ERROR_VALIDATION_FAILURE;
        }
        props.layer_name.fill(0);
        copy_str(&mut props.layer_name, LAYER_NAME);
        props.spec_version = XR_CURRENT_API_VERSION;
        props.layer_version = LAYER_IMPL_VERSION;
        // Quirk kept from C++: the description is copied without clearing the
        // rest of the buffer first.
        copy_str(&mut props.description, LAYER_DESCRIPTION);
        xr::Result::SUCCESS
    })
}

#[allow(unsafe_code)]
pub(crate) unsafe extern "system" fn xr_enumerate_instance_extension_properties(
    layer_name: *const c_char,
    property_capacity_input: u32,
    property_count_output: *mut u32,
    properties: *mut xr::ExtensionProperties,
) -> xr::Result {
    guard(|| {
        // SAFETY: a non-null `layer_name` is NUL-terminated.
        let is_ours = !layer_name.is_null()
            && unsafe { CStr::from_ptr(layer_name) }.to_bytes() == LAYER_NAME.as_bytes();
        if is_ours {
            // We don't implement any instance extensions.
            // SAFETY: the caller passes a valid count pointer.
            unsafe { *property_count_output = 0 };
            return xr::Result::SUCCESS;
        }
        let next_fn = STATE
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|s| s.next.enumerate_instance_extension_properties);
        if let Some(f) = next_fn {
            // SAFETY: the caller's arguments, passed through.
            return unsafe {
                f(
                    layer_name,
                    property_capacity_input,
                    property_count_output,
                    properties,
                )
            };
        }
        if !layer_name.is_null() {
            // Not ours, so it should be an earlier layer, or we'd have a next.
            return xr::Result::ERROR_API_LAYER_NOT_PRESENT;
        }
        // For a null layer name we append our (empty) list to the next's.
        // SAFETY: the caller passes a valid count pointer.
        unsafe { *property_count_output = 0 };
        xr::Result::SUCCESS
    })
}

#[allow(unsafe_code)]
pub(crate) unsafe extern "system" fn xr_get_instance_proc_addr(
    instance: xr::Instance,
    name: *const c_char,
    function: *mut Option<pfn::VoidFunction>,
) -> xr::Result {
    guard(|| {
        // SAFETY: the caller passes a NUL-terminated name.
        let name_str = unsafe { CStr::from_ptr(name) };
        let hooked = match name_str.to_bytes() {
            b"xrCreateSession" => void_fn!(xr_create_session, pfn::CreateSession),
            b"xrDestroySession" => void_fn!(xr_destroy_session, pfn::DestroySession),
            b"xrDestroyInstance" => void_fn!(xr_destroy_instance, pfn::DestroyInstance),
            b"xrEndFrame" => void_fn!(xr_end_frame, pfn::EndFrame),
            b"xrEnumerateApiLayerProperties" => {
                void_fn!(
                    xr_enumerate_api_layer_properties,
                    pfn::EnumerateApiLayerProperties
                )
            }
            b"xrEnumerateInstanceExtensionProperties" => void_fn!(
                xr_enumerate_instance_extension_properties,
                pfn::EnumerateInstanceExtensionProperties
            ),
            _ => None,
        };
        if hooked.is_some() {
            // SAFETY: the caller passes a valid out-pointer.
            unsafe { *function = hooked };
            return xr::Result::SUCCESS;
        }
        let next_gipa = STATE
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.next.get_instance_proc_addr);
        if let Some(gipa) = next_gipa {
            // SAFETY: the caller's arguments, passed through.
            return unsafe { gipa(instance, name, function) };
        }
        log(&format!(
            "Unsupported OpenXR call '{}' with instance {:#018x} and no next",
            name_str.to_string_lossy(),
            instance.into_raw()
        ));
        xr::Result::ERROR_FUNCTION_UNSUPPORTED
    })
}

#[allow(unsafe_code)]
pub(crate) unsafe extern "system" fn xr_create_api_layer_instance(
    info: *const xr::InstanceCreateInfo,
    layer_info: *const xr::ApiLayerCreateInfo,
    instance: *mut xr::Instance,
) -> xr::Result {
    guard(|| {
        // SAFETY: the loader passes valid structs, and `next_info` points at
        // this layer's entry in the chain.
        let (info_ref, layer_info, our_next) =
            unsafe { (&*info, &*layer_info, &*(*layer_info).next_info) };
        let app = &info_ref.application_info;
        log(&format!(
            "xrCreateApiLayerInstance - Application `{}` (v{}), engine `{}` (v{})",
            bounded(&app.application_name),
            app.application_version,
            bounded(&app.engine_name),
            app.engine_version
        ));
        let mut next_layer_info = *layer_info;
        next_layer_info.next_info = our_next.next;
        let create = our_next
            .next_create_api_layer_instance
            .expect("nextCreateApiLayerInstance");
        // SAFETY: the caller's arguments, with our chain entry stripped.
        let ret = unsafe { create(info, &next_layer_info, instance) };
        if failed(ret) {
            log(&format!("Next xrCreateApiLayerInstance failed: {ret:?}"));
            return ret;
        }

        for i in 0..info_ref.enabled_extension_count as usize {
            // SAFETY: the caller passes `enabled_extension_count` valid names.
            let ext = unsafe { CStr::from_ptr(*info_ref.enabled_extension_names.add(i)) };
            let ext = ext.to_string_lossy();
            log(&format!("Application enabled extension: {ext}"));
            if ext == "XR_KHR_composition_layer_depth" {
                log("Application enabled XR_KHR_COMPOSITION_LAYER_DEPTH; runtime bugs are likely");
            }
        }

        let gipa = our_next
            .next_get_instance_proc_addr
            .expect("nextGetInstanceProcAddr");
        // SAFETY: the next layer's `xrGetInstanceProcAddr` and the instance it
        // just created.
        let next = unsafe { Next::load(*instance, gipa) };
        let mut state = STATE.lock().unwrap();
        // Like C++, a new instance replaces `Next()` but keeps the runtime and
        // kneeboard globals.
        match state.as_mut() {
            Some(s) => s.next = next,
            None => {
                *state = Some(LayerState {
                    next,
                    runtime_name: String::new(),
                    kneeboard: None,
                })
            }
        }
        log("Created API layer instance");
        xr::Result::SUCCESS
    })
}

/// The layer's only export, found by the loader through the layer JSON.
///
/// # Safety
/// Called by the OpenXR loader: `layer_name` is NUL-terminated and
/// `api_layer_request` is valid.
#[allow(non_snake_case, unsafe_code)]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn OpenKneeboard_xrNegotiateLoaderApiLayerInterface(
    _loader_info: *const xr::NegotiateLoaderInfo,
    layer_name: *const c_char,
    api_layer_request: *mut xr::NegotiateApiLayerRequest,
) -> xr::Result {
    guard(|| {
        log("OpenKneeboard_xrNegotiateLoaderApiLayerInterface");
        // SAFETY: the loader passes a NUL-terminated name.
        let name = unsafe { CStr::from_ptr(layer_name) };
        if name.to_bytes() != LAYER_NAME.as_bytes() {
            log(&format!(
                "Layer name mismatch:\n -{LAYER_NAME}\n +{}",
                name.to_string_lossy()
            ));
            return xr::Result::ERROR_INITIALIZATION_FAILED;
        }
        // SAFETY: the loader passes a valid request struct.
        let request = unsafe { &mut *api_layer_request };
        request.layer_interface_version = XR_CURRENT_LOADER_API_LAYER_VERSION;
        request.layer_api_version = XR_CURRENT_API_VERSION;
        request.get_instance_proc_addr = Some(xr_get_instance_proc_addr);
        request.create_api_layer_instance = Some(xr_create_api_layer_instance);
        xr::Result::SUCCESS
    })
}
