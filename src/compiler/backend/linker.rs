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
        let is_compiler_clang = self.is_clang_compatible(&compiler);

        let mut final_objects = Vec::new();

        fn cleanup_file(path: &std::path::Path) {
            let _ = std::fs::remove_file(path);
        }

        struct ObjectCleanupGuard(Vec<std::path::PathBuf>);
        impl Drop for ObjectCleanupGuard {
            fn drop(&mut self) {
                for obj in &self.0 {
                    cleanup_file(obj);
                }
            }
        }

        let mut generated_objects_guard = ObjectCleanupGuard(Vec::new());

        // Step 1: Compile any .ll files to object (.o) or assembly (.s)
        for obj in &self.obj_paths {
            if obj.extension().and_then(|s| s.to_str()) == Some("ll") {
                let asm_tool = if is_compiler_clang {
                    compiler.clone()
                } else if let Some(ll_tool) = self.find_llvm_assembler() {
                    ll_tool
                } else {
                    return Err(format!(
                        "Cannot assemble '{}': Clang or LLVM is required to compile LLVM IR (.ll) files.\n\
                         The compiler found was '{}', which does not support LLVM IR.\n\
                         Please install Clang:\n  \
                         Ubuntu/Debian: sudo apt-get update && sudo apt-get install -y clang\n  \
                         Fedora:        sudo dnf install -y clang\n  \
                         Arch Linux:    sudo pacman -S clang",
                        obj.display(),
                        compiler
                    ));
                };

                let is_tool_llc = asm_tool.contains("llc");

                if self.emit_asm {
                    let out_asm = if self.obj_paths.len() == 1 {
                        self.output_path.with_extension("s")
                    } else {
                        obj.with_extension("s")
                    };
                    let mut asm_cmd = Command::new(&asm_tool);
                    if is_tool_llc {
                        asm_cmd.arg("-filetype=asm");
                        asm_cmd.arg(&self.opt_level);
                        if let Some(ref target) = self.target {
                            asm_cmd.arg(format!("--mtriple={}", target));
                        }
                    } else {
                        asm_cmd.arg("-S");
                        asm_cmd.arg(&self.opt_level);
                        if self.debug {
                            asm_cmd.arg("-g");
                        }
                        if let Some(ref target) = self.target {
                            asm_cmd.arg(format!("--target={}", target));
                        }
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
                        cleanup_file(obj);
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

                // Assemble .ll to Object (.o)
                let mut obj_cmd = Command::new(&asm_tool);
                if is_tool_llc {
                    obj_cmd.arg("-filetype=obj");
                    obj_cmd.arg(&self.opt_level);
                    if let Some(ref target) = self.target {
                        obj_cmd.arg(format!("--mtriple={}", target));
                    }
                } else {
                    obj_cmd.arg("-c");
                    obj_cmd.arg(&self.opt_level);
                    if self.debug {
                        obj_cmd.arg("-g");
                    }
                    if let Some(ref target) = self.target {
                        obj_cmd.arg(format!("--target={}", target));
                    }
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
                    cleanup_file(&out_obj);
                    cleanup_file(obj);
                    return Err(format!(
                        "Assembly failed for {}:\n{}",
                        obj.display(),
                        stderr
                    ));
                }

                if !self.compile_only {
                    generated_objects_guard.0.push(out_obj.clone());
                }
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
        if let Some(ref target) = self.target {
            cmd.arg(format!("--target={}", target));
        }

        cmd.arg(&self.opt_level);
        if self.debug {
            cmd.arg("-g");
        }

        // Add library search directories first so linkers can find all libraries
        for dir in &self.lib_dirs {
            cmd.arg(format!("-L{}", dir.display()));
        }

        // Determine target platform for system libraries
        let is_linux = if let Some(ref target) = self.target {
            target.contains("linux")
        } else {
            cfg!(target_os = "linux")
        };

        let is_macos = if let Some(ref target) = self.target {
            target.contains("darwin") || target.contains("apple") || target.contains("macos")
        } else {
            cfg!(target_os = "macos")
        };

        if is_linux {
            // Use --start-group and --end-group to resolve any circular dependencies in static archives
            cmd.arg("-Wl,--start-group");
            for obj in &final_objects {
                cmd.arg(obj);
            }
            for lib in &self.libs {
                cmd.arg(format!("-l{}", lib));
            }
            cmd.arg("-lm");
            cmd.arg("-lpthread");
            cmd.arg("-ldl");
            let (ssl_flag, crypto_flag) = self.find_linux_ssl_flags();
            cmd.arg(ssl_flag);
            cmd.arg(crypto_flag);
            cmd.arg("-Wl,--end-group");
        } else {
            // Add compiled object files and static archives
            for obj in &final_objects {
                cmd.arg(obj);
            }

            // Add user-specified libraries
            for lib in &self.libs {
                cmd.arg(format!("-l{}", lib));
            }

            if is_macos {
                cmd.arg("-framework");
                cmd.arg("Security");
                cmd.arg("-framework");
                cmd.arg("CoreFoundation");
                cmd.arg("-framework");
                cmd.arg("SystemConfiguration");
            }
        }

        cmd.arg("-o");
        cmd.arg(&self.output_path);

        if self.verbose {
            eprintln!("[linker] Executing: {:?}", cmd);
        }

        let output = cmd
            .output()
            .map_err(|e| format!("Failed to execute linker {}: {}", compiler, e))?;

        drop(generated_objects_guard);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            cleanup_file(&self.output_path);

            if stderr.contains("cannot find -lssl")
                || stderr.contains("cannot find -lcrypto")
                || stderr.contains("cannot find -l:libssl")
                || stderr.contains("cannot find -l:libcrypto")
            {
                return Err(format!(
                    "Linker failed: OpenSSL libraries not found.\n\
                     Please install the OpenSSL development package:\n  \
                     Ubuntu/Debian: sudo apt-get update && sudo apt-get install -y libssl-dev\n  \
                     Fedora/RHEL:   sudo dnf install -y openssl-devel\n  \
                     Arch Linux:    sudo pacman -S openssl\n  \
                     Alpine:        sudo apk add openssl-dev\n\n\
                     Details:\n{}",
                    stderr
                ));
            }

            return Err(format!("Linker failed:\n{}", stderr));
        }

        Ok(())
    }

    fn find_compiler(&self) -> Result<String, String> {
        // Respect CC environment variable
        if let Ok(cc) = env::var("CC") {
            return Ok(cc);
        }

        // Check for compilers in order of preference (clang is preferred for LLVM IR .ll support)
        let candidates = [
            "clang",
            "clang-19",
            "clang-18",
            "clang-17",
            "clang-16",
            "clang-15",
            "cc",
            "gcc",
        ];
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

    fn is_clang_compatible(&self, cmd: &str) -> bool {
        if let Ok(output) = Command::new(cmd).arg("--version").output() {
            let stdout = String::from_utf8_lossy(&output.stdout).to_lowercase();
            let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
            stdout.contains("clang")
                || stdout.contains("llvm")
                || stderr.contains("clang")
                || stderr.contains("llvm")
        } else {
            false
        }
    }

    fn find_llvm_assembler(&self) -> Option<String> {
        let candidates = [
            "clang",
            "clang-19",
            "clang-18",
            "clang-17",
            "clang-16",
            "clang-15",
            "llc",
            "llc-19",
            "llc-18",
            "llc-17",
            "llc-16",
            "llc-15",
        ];
        for bin in candidates {
            if self.check_command(bin) {
                return Some(bin.to_string());
            }
        }
        None
    }

    fn find_linux_ssl_flags(&self) -> (&'static str, &'static str) {
        let search_paths = [
            "/usr/lib/x86_64-linux-gnu",
            "/usr/lib/aarch64-linux-gnu",
            "/usr/lib64",
            "/usr/lib",
            "/usr/local/lib",
            "/lib/x86_64-linux-gnu",
            "/lib/aarch64-linux-gnu",
            "/lib64",
            "/lib",
        ];

        // 1. If standard unversioned libssl.so exists (libssl-dev installed)
        for dir in &search_paths {
            if Path::new(dir).join("libssl.so").exists() {
                return ("-lssl", "-lcrypto");
            }
        }

        // 2. If libssl.so.3 exists (standard on Ubuntu 22+, Debian 12+, Fedora 36+)
        for dir in &search_paths {
            if Path::new(dir).join("libssl.so.3").exists() {
                return ("-l:libssl.so.3", "-l:libcrypto.so.3");
            }
        }

        // 3. If libssl.so.1.1 exists (Ubuntu 20, Debian 11, etc.)
        for dir in &search_paths {
            if Path::new(dir).join("libssl.so.1.1").exists() {
                return ("-l:libssl.so.1.1", "-l:libcrypto.so.1.1");
            }
        }

        // Fallback default
        ("-lssl", "-lcrypto")
    }
}
