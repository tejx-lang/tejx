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
        
        let mut final_objects = Vec::new();

        // Step 1: Compile any .ll files to .s (assembly) to bypass Apple Clang object emitter bugs, then assemble to .o
        for obj in &self.obj_paths {
            if obj.extension().and_then(|s| s.to_str()) == Some("ll") {
                let out_asm = obj.with_extension("s");
                let out_obj = obj.with_extension("o");

                // Generate Assembly (.s)
                let mut asm_cmd = Command::new(&compiler);
                asm_cmd.arg("-S");
                asm_cmd.arg("-O3"); // Disable all optimizations to bypass ARM64 Clang driver crash
                asm_cmd.arg(obj);
                asm_cmd.arg("-o");
                asm_cmd.arg(&out_asm);

                let output_asm = asm_cmd.output().map_err(|e| format!("Failed to generate assembly {}: {}", obj.display(), e))?;
                if !output_asm.status.success() {
                    let stderr = String::from_utf8_lossy(&output_asm.stderr);
                    return Err(format!("LLVM assembly generation failed for {}:\n{}", obj.display(), stderr));
                }

                // Assemble to Object (.o)
                let mut obj_cmd = Command::new(&compiler);
                obj_cmd.arg("-c");
                obj_cmd.arg(&out_asm);
                obj_cmd.arg("-o");
                obj_cmd.arg(&out_obj);

                let output_obj = obj_cmd.output().map_err(|e| format!("Failed to assemble {}: {}", out_asm.display(), e))?;
                if !output_obj.status.success() {
                    let stderr = String::from_utf8_lossy(&output_obj.stderr);
                    return Err(format!("Assembly failed for {}:\n{}", out_asm.display(), stderr));
                }
                
                // Cleanup intermediate .s
                let _ = std::fs::remove_file(&out_asm);

                final_objects.push(out_obj);
            } else {
                final_objects.push(obj.to_path_buf());
            }
        }

        // Step 2: Link objects and libraries into final executable
        let mut cmd = Command::new(&compiler);
        cmd.arg("-O3"); // Linker optimizations

        for obj in &final_objects {
            cmd.arg(obj);
        }

        cmd.arg("-o");
        cmd.arg(&self.output_path);

        if cfg!(target_os = "linux") {
            cmd.arg("-lm");
            cmd.arg("-lpthread");
            cmd.arg("-ldl");
        }

        for lib in &self.libs {
            cmd.arg(format!("-l{}", lib));
        }

        let output = cmd.output().map_err(|e| format!("Failed to execute linker {}: {}", compiler, e))?;

        // Cleanup intermediate .o files to prevent disk clutter
        for obj in &final_objects {
            if obj.extension().and_then(|s| s.to_str()) == Some("o") {
                let _ = std::fs::remove_file(obj);
            }
        }

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
