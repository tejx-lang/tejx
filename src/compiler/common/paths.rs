use std::path::PathBuf;

/// Centralized path constants and resolution for the TejX toolchain.
///
/// Deployment layout:
///   /usr/local/tejx/
///   ├── bin/tejxc
///   ├── runtime/tejx_rt.a
///   └── lib/
///       ├── core/   (prelude.tx, array.tx, string.tx, object.tx)
///       └── std/    (math.tx, collections.tx, fs.tx, ...)

// Default subdirectory names
pub const LIB_DIR: &str = "lib";
pub const CORE_DIR: &str = "core";
pub const STD_DIR: &str = "std";
pub const RUNTIME_DIR: &str = "runtime";
pub const RUNTIME_LIB_NAME: &str = "tejx_rt.a";


// Environment variable names
// (REMOVED: TEJX_STDLIB_PATH and TEJX_RUNTIME_PATH are no longer supported)

/// Resolve the stdlib (lib/) directory path.
/// Priority: explicit > local (lib/) > installed (relative to binary)
pub fn resolve_stdlib_path(explicit: Option<&str>) -> PathBuf {
    // Priority: explicit > local (lib/) > installed (relative to binary)
    if let Some(p) = explicit {
        return PathBuf::from(p);
    }

    // Local lib/ directory
    if std::path::Path::new(LIB_DIR).exists() {
        return PathBuf::from(LIB_DIR);
    }
    // Installed mode: /usr/local/tejx/bin/tejxc → /usr/local/tejx/lib
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            let installed_lib = bin_dir.parent().map(|p| p.join(LIB_DIR)).unwrap_or_default();
            if installed_lib.exists() {
                return installed_lib;
            }
        }
    }
    // Fallback
    PathBuf::from(LIB_DIR)
}

/// Resolve the runtime library path.
/// Priority: explicit > installed (relative to binary)
pub fn resolve_runtime_path(explicit: Option<&str>) -> PathBuf {
    // Priority: explicit > installed (relative to binary)
    if let Some(p) = explicit {
        return PathBuf::from(p);
    }

    // Installed mode
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            let bin_parent = bin_dir.parent().unwrap_or(bin_dir);
            let installed_rt = bin_parent.join(RUNTIME_DIR).join(RUNTIME_LIB_NAME);
            if installed_rt.exists() {
                return installed_rt;
            }
            // Also check next to binary
            let local_rt = bin_dir.join(RUNTIME_LIB_NAME);
            if local_rt.exists() {
                return local_rt;
            }
        }
    }
    PathBuf::from(RUNTIME_DIR).join(RUNTIME_LIB_NAME)
}
