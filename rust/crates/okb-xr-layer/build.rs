fn main() {
    println!("cargo:rerun-if-changed=OpenKneeboard-OpenXR-Rust.def");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        println!("cargo:rustc-cdylib-link-arg=/DEF:{dir}/OpenKneeboard-OpenXR-Rust.def");
    }
}
