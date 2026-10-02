use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Linker {
    output_path: PathBuf,
    obj_paths: Vec<PathBuf>,
    libs: Vec<String>,
    lib_dirs: Vec<PathBuf>,
    target: Option<String>,
    compile_only: bool,
    opt_level: String,
    debug: bool,
    emit_asm: bool,
    verbose: bool,
}

impl Linker {
    pub fn new(output_path: &Path) -> Self {
        Self {
            output_path: output_path.to_path_buf(),
            obj_paths: Vec::new(),
            libs: Vec::new(),
            lib_dirs: Vec::new(),
            target: None,
            compile_only: false,
            opt_level: "-O3".to_string(),
            debug: false,
            emit_asm: false,
            verbose: false,
        }
    }

    pub fn set_compile_only(&mut self, compile_only: bool) {
        self.compile_only = compile_only;
    }

    pub fn set_opt_level(&mut self, opt_level: &str) {
        self.opt_level = opt_level.to_string();
    }

    pub fn set_debug(&mut self, debug: bool) {
        self.debug = debug;
    }

    pub fn set_emit_asm(&mut self, emit_asm: bool) {
        self.emit_asm = emit_asm;
    }

    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    pub fn add_lib(&mut self, lib: &str) {
        self.libs.push(lib.to_string());
    }

    pub fn add_lib_dir(&mut self, dir: &Path) {
        self.lib_dirs.push(dir.to_path_buf());
    }

    pub fn set_target(&mut self, target: &str) {
        self.target = Some(target.to_string());
    }

    pub fn add_object(&mut self, path: &Path) {
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_file() {
                        let ext = p.extension().and_then(|s| s.to_str());
                        if ext == Some("o")
                            || ext == Some("a")
                            || ext == Some("so")
                            || ext == Some("dylib")
                        {
                            self.obj_paths.push(p);
                        }
                    }
                }
            }
        } else {
            self.obj_paths.push(path.to_path_buf());
        }
    }

    pub fn link(&self) -> Result<(), String> {
        let compiler = self.find_compiler()?;

        let mut final_objects = Vec::new();

        struct CleanupGuard<'a>(&'a std::path::Path);
        impl<'a> Drop for CleanupGuard<'a> {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(self.0);
            }
        }

        let mut generated_objects = Vec::new();

        // Step 1: Compile any .ll files to .s (assembly) to bypass Apple Clang object emitter bugs, then assemble to .o
        for obj in &self.obj_paths {
            if obj.extension().and_then(|s| s.to_str()) == Some("ll") {
                if self.emit_asm {
                    let out_asm = if self.obj_paths.len() == 1 {
                        self.output_path.with_extension("s")
                    } else {
                        obj.with_extension("s")
                    };
                    let mut asm_cmd = Command::new(&compiler);
                    asm_cmd.arg("-S");
                    asm_cmd.arg(&self.opt_level);
                    if self.debug {
                        asm_cmd.arg("-g");
                    }
                    asm_cmd.arg(obj);
                    asm_cmd.arg("-o");
                    asm_cmd.arg(&out_asm);

                    if self.verbose {
                        eprintln!("[linker] Executing: {:?}", asm_cmd);
                    }

                    let output_asm = asm_cmd
                        .output()
                        .map_err(|e| format!("Failed to generate assembly {}: {}", obj.display(), e))?;
                    if !output_asm.status.success() {
                        let stderr = String::from_utf8_lossy(&output_asm.stderr);
                        return Err(format!(
                            "LLVM assembly generation failed for {}:\n{}",
                            obj.display(),
                            stderr
                        ));
                    }
                    continue;
                }

                let out_obj = if self.compile_only && self.obj_paths.len() == 1 {
                    self.output_path.with_extension("o")
                } else {
                    obj.with_extension("o")
                };

                // Directly assemble .ll to Object (.o)
                let mut obj_cmd = Command::new(&compiler);
                obj_cmd.arg("-c");
                obj_cmd.arg(&self.opt_level);
                if self.debug {
                    obj_cmd.arg("-g");
                }
                obj_cmd.arg(obj);
                obj_cmd.arg("-o");
                obj_cmd.arg(&out_obj);

                if self.verbose {
                    eprintln!("[linker] Executing: {:?}", obj_cmd);
                }

                let output_obj = obj_cmd
                    .output()
                    .map_err(|e| format!("Failed to assemble {}: {}", obj.display(), e))?;

                if !output_obj.status.success() {
                    let stderr = String::from_utf8_lossy(&output_obj.stderr);
                    return Err(format!(
                        "Assembly failed for {}:\n{}",
                        obj.display(),
                        stderr
                    ));
                }

                generated_objects.push(out_obj.clone());
                final_objects.push(out_obj);
            } else {
                final_objects.push(obj.to_path_buf());
            }
        }

        if self.emit_asm {
            return Ok(());
        }

        if self.compile_only {
            return Ok(());
        }

        // Step 2: Link objects and libraries into final executable
        let mut cmd = Command::new(&compiler);
        cmd.arg(&self.opt_level);
        if self.debug {
            cmd.arg("-g");
        }

        for obj in &final_objects {
            cmd.arg(obj);
        }

        cmd.arg("-o");
        cmd.arg(&self.output_path);

        if self.verbose {
            eprintln!("[linker] Executing: {:?}", cmd);
        }

        if cfg!(target_os = "linux") {
            cmd.arg("-lm");
            cmd.arg("-lpthread");
            cmd.arg("-ldl");
        } else if cfg!(target_os = "macos") {
            cmd.arg("-framework");
            cmd.arg("Security");
            cmd.arg("-framework");
            cmd.arg("CoreFoundation");
            cmd.arg("-framework");
            cmd.arg("SystemConfiguration");
        }

        if let Some(ref target) = self.target {
            cmd.arg(format!("--target={}", target));
        }

        for dir in &self.lib_dirs {
            cmd.arg(format!("-L{}", dir.display()));
        }

        for lib in &self.libs {
            cmd.arg(format!("-l{}", lib));
        }

        let output = cmd
            .output()
            .map_err(|e| format!("Failed to execute linker {}: {}", compiler, e))?;

        // Cleanup intermediate .o files to prevent disk clutter
        for obj in &generated_objects {
            let _ = std::fs::remove_file(obj);
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
