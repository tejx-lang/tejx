fn main() {
    // The WASM crate doesn't need to compile a native runtime library.
    // All runtime functions are provided by the JS host via WASM imports.
    println!("cargo:rerun-if-changed=src/");
    println!("cargo:rerun-if-changed=../src/");
}
