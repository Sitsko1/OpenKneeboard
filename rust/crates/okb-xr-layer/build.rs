use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=OpenKneeboard-OpenXR-Rust.def");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        println!("cargo:rustc-cdylib-link-arg=/DEF:{dir}/OpenKneeboard-OpenXR-Rust.def");
    }
    compile_sprite_shaders();
}

/// fxc from `OKB_FXC` (set by CMake), else the newest Windows 10 SDK's x64 fxc.
fn find_fxc() -> PathBuf {
    println!("cargo:rerun-if-env-changed=OKB_FXC");
    if let Some(fxc) = std::env::var_os("OKB_FXC") {
        return fxc.into();
    }
    let root = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"))
        .join(r"Windows Kits\10\bin");
    let mut sdks: Vec<_> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let version: Vec<u32> = e
                .file_name()
                .to_str()?
                .split('.')
                .map(str::parse)
                .collect::<Result<_, _>>()
                .ok()?;
            let fxc = e.path().join(r"x64\fxc.exe");
            fxc.is_file().then_some((version, fxc))
        })
        .collect();
    sdks.sort();
    sdks.pop()
        .map(|(_, fxc)| fxc)
        .expect("fxc not found: set OKB_FXC, or install a Windows 10 SDK with x64\\fxc.exe")
}

/// `src/shaders/Sprite.hlsl`, with the fxc flags from `src/shaders/CMakeLists.txt`.
fn compile_sprite_shaders() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let source = manifest.join("../../../src/shaders/Sprite.hlsl");
    println!("cargo:rerun-if-changed={}", source.display());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let fxc = find_fxc();
    for (profile, entry, file) in [
        ("vs_5_0", "SpriteVertexShader", "sprite_vs.dxbc"),
        ("ps_5_0", "SpritePixelShader", "sprite_ps.dxbc"),
    ] {
        let status = Command::new(&fxc)
            .args(["/nologo", "/T", profile, "/E", entry, "/Fo"])
            .arg(out.join(file))
            .arg(&source)
            .status()
            .unwrap_or_else(|e| panic!("failed to run {}: {e}", fxc.display()));
        assert!(status.success(), "fxc failed for {entry}: {status}");
    }
}
