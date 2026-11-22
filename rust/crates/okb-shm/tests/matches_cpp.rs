//! Fails when a wire constant changes on one side of the C++/Rust boundary only.

/// Finds the line containing `<name> =` and parses the integer before `;`.
fn cpp_const(src: &str, name: &str) -> usize {
    let needle = format!("{name} =");
    let line = src
        .lines()
        .find(|l| l.contains(&needle))
        .unwrap_or_else(|| panic!("{name} not found in C++ source"));
    let value = line
        .split_once(&needle)
        .and_then(|(_, rest)| rest.split_once(';'))
        .map(|(v, _)| v.trim())
        .unwrap_or_else(|| panic!("{name}: no `= <value>;` on line `{line}`"));
    value
        .parse()
        .unwrap_or_else(|e| panic!("{name}: `{value}` is not an integer: {e}"))
}

#[test]
fn constants_match_cpp_config() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../src/lib/include/OpenKneeboard/config.in.hpp"
    ))
    .expect("C++ config header readable");
    assert_eq!(
        cpp_const(&src, "SHMSwapchainLength"),
        okb_shm::SWAPCHAIN_LENGTH,
        "SHMSwapchainLength drifted"
    );
    assert_eq!(
        cpp_const(&src, "MaxViewCount"),
        okb_shm::MAX_VIEW_COUNT,
        "MaxViewCount drifted"
    );
}
