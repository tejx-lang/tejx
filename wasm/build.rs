use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src/");
    println!("cargo:rerun-if-changed=../src/library/");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("embedded_modules.rs");

    let lib_dir = Path::new("../src/library");
    let mut entries = Vec::new();

    fn visit_dir(dir: &Path, base: &Path, entries: &mut Vec<(String, PathBuf)>) {
        if let Ok(rd) = fs::read_dir(dir) {
            let mut paths: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            paths.sort();
            for p in paths {
                if p.is_dir() {
                    visit_dir(&p, base, entries);
                } else if p.extension().map_or(false, |ext| ext == "tx") {
                    let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                    entries.push((rel, p));
                }
            }
        }
    }

    visit_dir(lib_dir, lib_dir, &mut entries);

    let mut generated = String::new();
    generated.push_str("fn builtin_modules() -> HashMap<String, String> {\n");
    generated.push_str("    let mut m = HashMap::new();\n");
    for (rel, abs) in entries {
        let _abs_str = abs.to_string_lossy().replace('\\', "/");
        if rel == "core/prelude.tx" {
            // Keep prelude clean in WASM without heavy native server/networking modules
            generated.push_str("    m.insert(\"core/prelude.tx\".to_string(), \"import \\\"./base.tx\\\";\\n\".to_string());\n");
        } else if rel == "core/base.tx" {
            let mut content = fs::read_to_string(&abs).unwrap();
            content = content.replace("constructor(msg: string, code: Optional<int>)", "constructor(msg: string)");
            content = content.replace("this.code = code ?? 0;", "this.code = 0;");
            content = content.replace("super(msg, None);", "super(msg);");
            content = content.replace("super(msg, 0);", "super(msg);");
            let out_base = Path::new(&out_dir).join("wasm_base.tx");
            fs::write(&out_base, content).unwrap();
            generated.push_str("    m.insert(\"core/base.tx\".to_string(), include_str!(concat!(env!(\"OUT_DIR\"), \"/wasm_base.tx\")).to_string());\n");
        } else {
            generated.push_str(&format!(
                "    m.insert(\"{}\".to_string(), include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../src/library/{}\")).to_string());\n",
                rel, rel
            ));
        }
        if rel.starts_with("std/") && rel.chars().filter(|&c| c == '/').count() == 1 {
            let stem = &rel["std/".len()..rel.len() - 3];
            generated.push_str(&format!(
                "    m.insert(\"std:{}\".to_string(), m.get(\"{}\").unwrap().clone());\n",
                stem, rel
            ));
        }
    }
    generated.push_str("    m\n");
    generated.push_str("}\n");

    fs::write(dest_path, generated).unwrap();
}
