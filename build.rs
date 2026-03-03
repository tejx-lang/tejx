use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/runtime");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_lib = Path::new(&out_dir).join("libruntime.a");

    // Compile src/runtime/mod.rs to a static library
    let status = Command::new("rustc")
        .arg("--crate-type=staticlib")
        .arg("src/runtime/mod.rs")
        .arg("-C")
        .arg("opt-level=3")
        .arg("-C")
        .arg("panic=abort")
        .arg("-o")
        .arg(&dest_lib)
        .status()
        .expect("Failed to run rustc");

    if !status.success() {
        panic!("Failed to compile src/runtime.rs to static library");
    }
}
