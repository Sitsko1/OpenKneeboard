//! A fake "next" layer/runtime for the in-crate tests: records what the layer
//! forwards so the tests can check it passed calls through untouched.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::ffi::{CStr, c_char};
use std::sync::{Mutex, MutexGuard, PoisonError};

use openxr_sys as xr;
use xr::pfn;

use crate::STATE;

pub(crate) const FAKE_INSTANCE: u64 = 0x1234;
pub(crate) const FAKE_RUNTIME_NAME: &str = "FakeRuntime";

#[derive(Default)]
pub(crate) struct Calls {
    /// `layerInfo->nextInfo` as seen by the next `xrCreateApiLayerInstance`.
    pub(crate) create_instance_next_info: Option<*mut xr::ApiLayerNextInfo>,
    pub(crate) create_session: u32,
    pub(crate) destroy_session: u32,
    pub(crate) destroy_instance: u32,
    /// `(frameEndInfo, layers, layerCount)` for each forwarded `xrEndFrame`.
    pub(crate) end_frame: Vec<(
        *const xr::FrameEndInfo,
        *const *const xr::CompositionLayerBaseHeader,
        u32,
    )>,
}

thread_local! {
    pub(crate) static CALLS: RefCell<Calls> = RefCell::default();
}

static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Serializes tests that touch the global layer state (needed under
/// `cargo test`; nextest already isolates tests in processes) and resets it.
pub(crate) fn setup() -> MutexGuard<'static, ()> {
    let guard = TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    *STATE.lock().unwrap() = None;
    CALLS.with_borrow_mut(|c| *c = Calls::default());
    guard
}

pub(crate) unsafe extern "system" fn fake_vulkan_device(
    _: xr::Instance,
    _: *const xr::VulkanDeviceCreateInfoKHR,
    _: *mut xr::platform::VkDevice,
    _: *mut xr::platform::VkResult,
) -> xr::Result {
    xr::Result::SUCCESS
}

unsafe extern "system" fn fake_create_session(
    _: xr::Instance,
    _: *const xr::SessionCreateInfo,
    session: *mut xr::Session,
) -> xr::Result {
    let n = CALLS.with_borrow_mut(|c| {
        c.create_session += 1;
        c.create_session
    });
    // SAFETY: the layer passes the caller's valid out-pointer through.
    unsafe { *session = xr::Session::from_raw(u64::from(n)) };
    xr::Result::SUCCESS
}

unsafe extern "system" fn fake_destroy_session(_: xr::Session) -> xr::Result {
    CALLS.with_borrow_mut(|c| c.destroy_session += 1);
    xr::Result::SUCCESS
}

unsafe extern "system" fn fake_destroy_instance(_: xr::Instance) -> xr::Result {
    CALLS.with_borrow_mut(|c| c.destroy_instance += 1);
    xr::Result::SUCCESS
}

unsafe extern "system" fn fake_end_frame(
    _: xr::Session,
    info: *const xr::FrameEndInfo,
) -> xr::Result {
    // SAFETY: the test passes a valid `FrameEndInfo`.
    let i = unsafe { &*info };
    CALLS.with_borrow_mut(|c| c.end_frame.push((info, i.layers, i.layer_count)));
    xr::Result::SUCCESS
}

unsafe extern "system" fn fake_get_instance_properties(
    _: xr::Instance,
    props: *mut xr::InstanceProperties,
) -> xr::Result {
    // SAFETY: the layer passes a valid `InstanceProperties`.
    let props = unsafe { &mut *props };
    for (dst, src) in props.runtime_name.iter_mut().zip(FAKE_RUNTIME_NAME.bytes()) {
        *dst = src as c_char;
    }
    xr::Result::SUCCESS
}

pub(crate) unsafe extern "system" fn fake_get_instance_proc_addr(
    _: xr::Instance,
    name: *const c_char,
    function: *mut Option<pfn::VoidFunction>,
) -> xr::Result {
    // SAFETY: callers pass a NUL-terminated name.
    let name = unsafe { CStr::from_ptr(name) }.to_str().unwrap();
    let f = match name {
        "xrCreateSession" => void_fn!(fake_create_session, pfn::CreateSession),
        "xrDestroySession" => void_fn!(fake_destroy_session, pfn::DestroySession),
        "xrDestroyInstance" => void_fn!(fake_destroy_instance, pfn::DestroyInstance),
        "xrEndFrame" => void_fn!(fake_end_frame, pfn::EndFrame),
        "xrGetInstanceProperties" => {
            void_fn!(fake_get_instance_properties, pfn::GetInstanceProperties)
        }
        "xrCreateVulkanDeviceKHR" => void_fn!(fake_vulkan_device, pfn::CreateVulkanDeviceKHR),
        _ => None,
    };
    // SAFETY: callers pass a valid out-pointer.
    unsafe { *function = f };
    if f.is_some() {
        xr::Result::SUCCESS
    } else {
        xr::Result::ERROR_FUNCTION_UNSUPPORTED
    }
}

pub(crate) unsafe extern "system" fn fake_create_api_layer_instance(
    _: *const xr::InstanceCreateInfo,
    layer_info: *const xr::ApiLayerCreateInfo,
    instance: *mut xr::Instance,
) -> xr::Result {
    // SAFETY: the layer passes a valid `ApiLayerCreateInfo` and out-pointer.
    unsafe {
        let next_info = (*layer_info).next_info;
        CALLS.with_borrow_mut(|c| c.create_instance_next_info = Some(next_info));
        *instance = xr::Instance::from_raw(FAKE_INSTANCE);
    }
    xr::Result::SUCCESS
}

/// A two-entry `XrApiLayerNextInfo` chain: `[0]` is ours (pointing at the
/// fake), `[1]` stands in for the layer after the fake.
pub(crate) fn next_info_chain() -> Box<[xr::ApiLayerNextInfo; 2]> {
    let entry = xr::ApiLayerNextInfo {
        struct_type: xr::LoaderInterfaceStructs::from_raw(5),
        struct_version: 1,
        struct_size: size_of::<xr::ApiLayerNextInfo>(),
        layer_name: [0; xr::MAX_API_LAYER_NAME_SIZE],
        next_get_instance_proc_addr: Some(fake_get_instance_proc_addr),
        next_create_api_layer_instance: Some(fake_create_api_layer_instance),
        next: std::ptr::null_mut(),
    };
    let mut chain = Box::new([entry, entry]);
    chain[0].next = &mut chain[1];
    chain
}

/// Runs the layer's `xrCreateApiLayerInstance` against the fake next.
pub(crate) fn create_instance() -> (xr::Result, xr::Instance, Box<[xr::ApiLayerNextInfo; 2]>) {
    let mut chain = next_info_chain();
    // SAFETY: all-zero is a valid `InstanceCreateInfo`/`ApiLayerCreateInfo`
    // (null pointers, zero counts, empty strings).
    let (mut info, mut layer_info): (xr::InstanceCreateInfo, xr::ApiLayerCreateInfo) =
        unsafe { (std::mem::zeroed(), std::mem::zeroed()) };
    info.ty = xr::StructureType::INSTANCE_CREATE_INFO;
    layer_info.struct_type = xr::LoaderInterfaceStructs::from_raw(4);
    layer_info.next_info = &mut chain[0];
    let mut instance = xr::Instance::NULL;
    // SAFETY: valid pointers to live locals.
    let ret = unsafe { crate::xr_create_api_layer_instance(&info, &layer_info, &mut instance) };
    (ret, instance, chain)
}

pub(crate) fn kneeboard_device() -> Option<usize> {
    STATE
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .kneeboard
        .as_ref()
        .map(|k| k.device)
}
