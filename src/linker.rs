use std::path::{Path, PathBuf};
use std::process::Command;
use std::env;

pub struct Linker {
    output_path: PathBuf,
    obj_paths: Vec<PathBuf>,
    libs: Vec<String>,
}

impl Linker {
    pub fn new(output_path: &Path) -> Self {
        Self {
            output_path: output_path.to_path_buf(),
            obj_paths: Vec::new(),
            libs: Vec::new(),
        }
    }

    pub fn add_object(&mut self, path: &Path) {
        self.obj_paths.push(path.to_path_buf());
    }

    #[allow(dead_code)]
    pub fn add_lib(&mut self, lib: &str) {
        self.libs.push(lib.to_string());
    }

    pub fn link(&self) -> Result<(), String> {
        let compiler = self.find_compiler()?;
        
        let mut cmd = Command::new(&compiler);
        cmd.arg("-O3"); // Optimize final binary

        // Input objects
        for obj in &self.obj_paths {
            cmd.arg(obj);
        }

        // Output file
        cmd.arg("-o");
        cmd.arg(&self.output_path);

        // Standard Libraries
        // Note: compiler drivers (cc, clang) link libc automatically usually
        if cfg!(target_os = "linux") {
            cmd.arg("-lm");
            cmd.arg("-lpthread");
            cmd.arg("-ldl");
        } else if cfg!(target_os = "macos") {
            // macOS clang links m, pthread, dl automatically or they are part of libSystem
            // But being explicit doesn't hurt for -lm usually
            // cmd.arg("-lm"); 
        }

        // Additional libs
        for lib in &self.libs {
            cmd.arg(format!("-l{}", lib));
        }

        let output = cmd.output().map_err(|e| format!("Failed to execute {}: {}", compiler, e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Linker failed:\n{}", stderr));
        }

        Ok(())
    }

    fn find_compiler(&self) -> Result<String, String> {
        // Respect CC environment variable
        if let Ok(cc) = env::var("CC") {
            return Ok(cc);
        }

        // Check for compilers in order of preference
        let candidates = ["cc", "clang", "gcc"];
        for bin in candidates {
            if self.check_command(bin) {
                return Ok(bin.to_string());
            }
        }

        // Fallback: No compiler found
        Err("No C/C++ compiler found. Please install a C compiler (e.g., clang, gcc, or cc) to proceed.".to_string())
    }

    fn check_command(&self, cmd: &str) -> bool {
        Command::new(cmd).arg("-v").output().is_ok()
    }
}
