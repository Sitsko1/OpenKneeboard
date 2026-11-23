#![allow(unsafe_code)]
use std::ffi::{CStr, c_char, c_void};
use std::ptr;

use openxr_sys as xr;
use xr::pfn;

use crate::identity::{LAYER_DESCRIPTION, LAYER_IMPL_VERSION, LAYER_NAME};
use crate::test_support::*;
use crate::*;

fn layer_name_c() -> std::ffi::CString {
    std::ffi::CString::new(LAYER_NAME).unwrap()
}

fn empty_request() -> xr::NegotiateApiLayerRequest {
    xr::NegotiateApiLayerRequest {
        struct_type: xr::LoaderInterfaceStructs::from_raw(2),
        struct_version: 1,
        struct_size: size_of::<xr::NegotiateApiLayerRequest>(),
        layer_interface_version: 0,
        layer_api_version: xr::Version::new(0, 0, 0),
        get_instance_proc_addr: None,
        create_api_layer_instance: None,
    }
}

fn c_str(chars: &[c_char]) -> &str {
    // SAFETY: the layer wrote a NUL-terminated string into a zeroed buffer.
    unsafe { CStr::from_ptr(chars.as_ptr()) }.to_str().unwrap()
}

fn create_session(binding: *const c_void) -> (xr::Result, xr::Session) {
    let info = xr::SessionCreateInfo {
        ty: xr::StructureType::SESSION_CREATE_INFO,
        next: binding,
        create_flags: xr::SessionCreateFlags::EMPTY,
        system_id: xr::SystemId::from_raw(1),
    };
    let mut session = xr::Session::NULL;
    // SAFETY: valid pointers to live locals.
    let ret =
        unsafe { xr_create_session(xr::Instance::from_raw(FAKE_INSTANCE), &info, &mut session) };
    (ret, session)
}

#[test]
fn negotiate_rejects_other_layer_name() {
    let _g = setup();
    let mut req = empty_request();
    // SAFETY: valid pointers to live locals.
    let ret = unsafe {
        OpenKneeboard_xrNegotiateLoaderApiLayerInterface(
            ptr::null(),
            c"XR_APILAYER_other".as_ptr(),
            &mut req,
        )
    };
    assert_eq!(ret, xr::Result::ERROR_INITIALIZATION_FAILED);
    assert!(req.get_instance_proc_addr.is_none());
}

#[test]
fn negotiate_fills_request() {
    let _g = setup();
    let mut req = empty_request();
    let name = layer_name_c();
    // SAFETY: valid pointers to live locals.
    let ret = unsafe {
        OpenKneeboard_xrNegotiateLoaderApiLayerInterface(ptr::null(), name.as_ptr(), &mut req)
    };
    assert_eq!(ret, xr::Result::SUCCESS);
    assert_eq!(req.layer_interface_version, 1);
    // C++ builds against OpenXR SDK 1.0.25: XR_CURRENT_API_VERSION.
    assert_eq!(req.layer_api_version, xr::Version::new(1, 0, 25));
    assert!(req.get_instance_proc_addr.is_some());
    assert!(req.create_api_layer_instance.is_some());
}

#[test]
fn enumerate_api_layer_properties_reports_one_layer() {
    let _g = setup();
    let mut count = 0;
    // SAFETY: capacity 0, so `properties` is not read.
    let ret = unsafe { xr_enumerate_api_layer_properties(0, &mut count, ptr::null_mut()) };
    assert_eq!((ret, count), (xr::Result::SUCCESS, 1));

    // SAFETY: all-zero is a valid `ApiLayerProperties`.
    let mut props: xr::ApiLayerProperties = unsafe { std::mem::zeroed() };
    // SAFETY: valid pointers to live locals.
    let ret = unsafe { xr_enumerate_api_layer_properties(1, &mut count, &mut props) };
    assert_eq!(ret, xr::Result::ERROR_VALIDATION_FAILURE);

    props.ty = xr::StructureType::API_LAYER_PROPERTIES;
    // SAFETY: valid pointers to live locals.
    let ret = unsafe { xr_enumerate_api_layer_properties(1, &mut count, &mut props) };
    assert_eq!(ret, xr::Result::SUCCESS);
    assert_eq!(c_str(&props.layer_name), LAYER_NAME);
    assert_eq!(c_str(&props.description), LAYER_DESCRIPTION);
    assert_eq!(props.spec_version, xr::Version::new(1, 0, 25));
    assert_eq!(props.layer_version, LAYER_IMPL_VERSION);
}

#[test]
fn enumerate_extensions_for_own_layer_is_empty() {
    let _g = setup();
    let name = layer_name_c();
    let mut count = 42;
    // SAFETY: valid pointers; `properties` is not read for our own layer.
    let ret = unsafe {
        xr_enumerate_instance_extension_properties(name.as_ptr(), 0, &mut count, ptr::null_mut())
    };
    assert_eq!((ret, count), (xr::Result::SUCCESS, 0));
}

#[test]
fn create_instance_chains_to_next() {
    let _g = setup();
    let (ret, instance, chain) = create_instance();
    assert_eq!(ret, xr::Result::SUCCESS);
    assert_eq!(instance.into_raw(), FAKE_INSTANCE);
    let seen = CALLS.with_borrow(|c| c.create_instance_next_info);
    assert_eq!(seen, Some(ptr::from_ref(&chain[1]).cast_mut()));
    assert!(STATE.lock().unwrap().is_some());
}

#[test]
fn d3d12_session_passes_through() {
    let _g = setup();
    let _chain = create_instance().2;
    let binding = xr::GraphicsBindingD3D12KHR {
        ty: xr::StructureType::GRAPHICS_BINDING_D3D12_KHR,
        next: ptr::null(),
        device: ptr::without_provenance_mut(0x1000),
        queue: ptr::without_provenance_mut(0x2000),
    };
    let (ret, session) = create_session(ptr::from_ref(&binding).cast());
    assert_eq!(ret, xr::Result::SUCCESS);
    assert_eq!(kneeboard_device(), None);
    assert_eq!(
        STATE.lock().unwrap().as_ref().unwrap().runtime_name,
        FAKE_RUNTIME_NAME
    );

    let layers: [*const xr::CompositionLayerBaseHeader; 2] = [ptr::null(); 2];
    let info = xr::FrameEndInfo {
        ty: xr::StructureType::FRAME_END_INFO,
        next: ptr::null(),
        display_time: xr::Time::from_nanos(1),
        environment_blend_mode: xr::EnvironmentBlendMode::OPAQUE,
        layer_count: 2,
        layers: layers.as_ptr(),
    };
    // SAFETY: valid pointer to a live local.
    let ret = unsafe { xr_end_frame(session, &info) };
    assert_eq!(ret, xr::Result::SUCCESS);
    let forwarded = CALLS.with_borrow(|c| c.end_frame.clone());
    assert_eq!(forwarded, vec![(ptr::from_ref(&info), layers.as_ptr(), 2)]);
}

#[test]
fn vulkan_functions_not_hooked() {
    let _g = setup();
    let _chain = create_instance().2;
    let mut f: Option<pfn::VoidFunction> = None;
    // SAFETY: valid pointers to live locals.
    let ret = unsafe {
        xr_get_instance_proc_addr(
            xr::Instance::from_raw(FAKE_INSTANCE),
            c"xrCreateVulkanDeviceKHR".as_ptr(),
            &mut f,
        )
    };
    assert_eq!(ret, xr::Result::SUCCESS);
    let expected = fake_vulkan_device as pfn::CreateVulkanDeviceKHR as usize;
    assert_eq!(f.map(|f| f as usize), Some(expected));
}

#[test]
fn hooked_functions_resolve_to_the_layer() {
    let _g = setup();
    let _chain = create_instance().2;
    let mut f: Option<pfn::VoidFunction> = None;
    // SAFETY: valid pointers to live locals.
    let ret =
        unsafe { xr_get_instance_proc_addr(xr::Instance::NULL, c"xrEndFrame".as_ptr(), &mut f) };
    assert_eq!(ret, xr::Result::SUCCESS);
    assert_eq!(
        f.map(|f| f as usize),
        Some(xr_end_frame as pfn::EndFrame as usize)
    );
}

#[test]
fn second_session_returns_limit_reached() {
    let _g = setup();
    let _chain = create_instance().2;
    let binding = xr::GraphicsBindingD3D11KHR {
        ty: xr::StructureType::GRAPHICS_BINDING_D3D11_KHR,
        next: ptr::null(),
        device: ptr::without_provenance_mut(0x1000),
    };
    let (ret, session) = create_session(ptr::from_ref(&binding).cast());
    assert_eq!(ret, xr::Result::SUCCESS);
    assert_eq!(kneeboard_device(), Some(0x1000));

    // C++ forwards first, then refuses.
    let (ret, _) = create_session(ptr::from_ref(&binding).cast());
    assert_eq!(ret, xr::Result::ERROR_LIMIT_REACHED);
    assert_eq!(CALLS.with_borrow(|c| c.create_session), 2);

    // SAFETY: plain handle.
    assert_eq!(unsafe { xr_destroy_session(session) }, xr::Result::SUCCESS);
    assert_eq!(kneeboard_device(), None);
    assert_eq!(CALLS.with_borrow(|c| c.destroy_session), 1);
}

#[test]
fn shm_names_use_identity() {
    assert_eq!(
        crate::identity::shm_names(),
        okb_shm::Names::new(
            "com.openkneeboard",
            okb_shm::Version {
                major: 1,
                minor: 13,
                patch: 0,
                build: 0
            }
        )
    );
}
