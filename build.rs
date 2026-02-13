use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let runtime_src = "src/runtime.rs";
    let runtime_lib = Path::new(&out_dir).join("libruntime.a");

    println!("cargo:rerun-if-changed={}", runtime_src);
    println!("cargo:rerun-if-changed=src/stdlib");

    // Compile src/runtime.rs into a static library
    let status = Command::new("rustc")
        .args(&["--crate-type=staticlib", "-O", "-g"]) // -g for debug symbols if needed, -O for optimized
        .arg("--emit=dep-info,link") // Ensure we get the .a file
        .arg("--cfg").arg("runtime_build")
        .arg("-o")
        .arg(&runtime_lib)
        .arg(runtime_src)
        .status()
        .expect("Failed to execute rustc for runtime library");

    if !status.success() {
        panic!("Failed to compile runtime library");
    }
}
