//! Function pointers into the next layer or runtime (C++ `OpenXRNext`).
#![allow(unsafe_code)]

use openxr_sys as xr;
use xr::pfn;

macro_rules! next_fns {
    ($($field:ident: $ty:ident = $name:literal,)*) => {
        /// The C++ `OPENKNEEBOARD_NEXT_OPENXR_FUNCS` list minus the Vulkan
        /// functions. Like C++, a missing function is `None` and only fails
        /// when called.
        #[derive(Clone, Copy)]
        pub(crate) struct Next {
            pub(crate) get_instance_proc_addr: pfn::GetInstanceProcAddr,
            $(pub(crate) $field: Option<pfn::$ty>,)*
        }

        impl Next {
            /// # Safety
            /// `get_instance_proc_addr` must be the next layer's valid
            /// `xrGetInstanceProcAddr`, and `instance` an instance it created.
            pub(crate) unsafe fn load(
                instance: xr::Instance,
                get_instance_proc_addr: pfn::GetInstanceProcAddr,
            ) -> Self {
                Self {
                    get_instance_proc_addr,
                    $($field: {
                        let mut f = None;
                        // SAFETY: valid NUL-terminated name and out-pointer;
                        // the caller guarantees the function pointer. The
                        // result is ignored as in C++: `f` stays `None`.
                        unsafe { get_instance_proc_addr(instance, $name.as_ptr(), &mut f) };
                        // SAFETY: the next layer returns the function named
                        // `$name`, whose signature is `pfn::$ty`.
                        f.map(|f| unsafe { std::mem::transmute::<pfn::VoidFunction, pfn::$ty>(f) })
                    },)*
                }
            }
        }
    };
}

next_fns! {
    create_session: CreateSession = c"xrCreateSession",
    destroy_session: DestroySession = c"xrDestroySession",
    destroy_instance: DestroyInstance = c"xrDestroyInstance",
    end_frame: EndFrame = c"xrEndFrame",
    acquire_swapchain_image: AcquireSwapchainImage = c"xrAcquireSwapchainImage",
    create_reference_space: CreateReferenceSpace = c"xrCreateReferenceSpace",
    create_swapchain: CreateSwapchain = c"xrCreateSwapchain",
    destroy_space: DestroySpace = c"xrDestroySpace",
    destroy_swapchain: DestroySwapchain = c"xrDestroySwapchain",
    enumerate_instance_extension_properties: EnumerateInstanceExtensionProperties =
        c"xrEnumerateInstanceExtensionProperties",
    enumerate_swapchain_formats: EnumerateSwapchainFormats = c"xrEnumerateSwapchainFormats",
    enumerate_swapchain_images: EnumerateSwapchainImages = c"xrEnumerateSwapchainImages",
    get_instance_properties: GetInstanceProperties = c"xrGetInstanceProperties",
    get_system_properties: GetSystemProperties = c"xrGetSystemProperties",
    locate_space: LocateSpace = c"xrLocateSpace",
    release_swapchain_image: ReleaseSwapchainImage = c"xrReleaseSwapchainImage",
    wait_swapchain_image: WaitSwapchainImage = c"xrWaitSwapchainImage",
}
