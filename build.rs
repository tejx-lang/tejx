use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=tejx-runtime/src");
    println!("cargo:rerun-if-changed=tejx-runtime/Cargo.toml");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_lib = Path::new(&out_dir).join("libruntime.a");

    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let runtime_target_dir = Path::new(&out_dir).join("runtime-target");

    let mut cmd = Command::new(&cargo);
    cmd.arg("build")
        .arg("--manifest-path=tejx-runtime/Cargo.toml")
        .arg("--target-dir")
        .arg(&runtime_target_dir);

    if profile != "debug" {
        cmd.arg("--release");
    }

    let status = cmd.status().expect("Failed to run cargo");

    if !status.success() {
        panic!("Failed to compile tejx-runtime to static library");
    }

    let profile_dir = if profile == "debug" {
        "debug"
    } else {
        "release"
    };
    let src_lib = runtime_target_dir
        .join(profile_dir)
        .join("libtejx_runtime.a");
    std::fs::copy(&src_lib, &dest_lib).expect("Failed to copy built library to out_dir");
}
