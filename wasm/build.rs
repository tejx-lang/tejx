use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let runtime_src = "../src/runtime.rs";
    let runtime_lib = Path::new(&out_dir).join("libruntime.a");

    let target = env::var("TARGET").unwrap_or_else(|_| "x86_64-apple-darwin".to_string());
    
    println!("cargo:rerun-if-changed={}", runtime_src);
    println!("cargo:rerun-if-changed=../src/stdlib");

    // Compile src/runtime.rs into a static library
    let mut args = vec!["--crate-type=staticlib", "-O", "-g", "--emit=dep-info,link", "--cfg", "runtime_build", "-o"];
    args.push(runtime_lib.to_str().unwrap());
    args.push(runtime_src);

    // If we're cross-compiling, pass the target
    if let Ok(host) = env::var("HOST") {
        if host != target {
            args.push("--target");
            args.push(&target);
        }
    }

    let status = Command::new("rustc")
        .args(&args)
        .status()
        .expect("Failed to execute rustc for runtime library");

    if !status.success() {
        panic!("Failed to compile runtime library");
    }
}
