use super::*;

impl TypeChecker {
    pub(crate) fn parse_signature(&self, type_name: String) -> (String, Vec<String>, bool) {
        let mut final_params = Vec::new();
        let mut final_type = type_name.clone();
        let mut is_variadic = false;

        let split_params = |params_str: &str| -> Vec<String> {
            let mut params = Vec::new();
            let mut current = String::new();
            let mut depth_brace = 0;
            let mut depth_angle = 0;
            let mut depth_paren = 0;

            for ch in params_str.chars() {
                match ch {
                    '{' => depth_brace += 1,
                    '}' => depth_brace -= 1,
                    '<' => depth_angle += 1,
                    '>' => {
                        if depth_angle > 0 {
                            depth_angle -= 1;
                        }
                    }
                    '(' => depth_paren += 1,
                    ')' => depth_paren -= 1,
                    ',' if depth_brace == 0 && depth_angle == 0 && depth_paren == 0 => {
                        params.push(current.trim().to_string());
                        current.clear();
                        continue;
                    }
                    _ => {}
                }
                current.push(ch);
            }
            if !current.trim().is_empty() {
                params.push(current.trim().to_string());
            }
            params
        };

        if type_name.starts_with("function:") {
            let parts: Vec<&str> = type_name.splitn(3, ':').collect();
            if parts.len() >= 3 {
                // function:ret_ty:p1,p2,p3
                final_type = format!("function:{}", parts[1]);
                let params = split_params(parts[2]);
                for mut p in params {
                    if p.ends_with("...") {
                        is_variadic = true;
                        p = p[..p.len() - 3].to_string();
                    }
                    if !p.is_empty() {
                        final_params.push(p);
                    }
                }
            }
        } else if type_name.contains("=>") {
            // (p1: t1, p2: t2) => ret
            if let Some(start) = type_name.find('(') {
                if let Some(end) = type_name.rfind(')') {
                    let params_str = &type_name[start + 1..end];
                    let params = split_params(params_str);
                    for p in params {
                        if p.ends_with("...") {
                            is_variadic = true;
                        }
                        let p = p.trim_end_matches("...").trim();
                        if let Some(colon) = p.find(':') {
                            final_params.push(p[colon + 1..].trim().to_string());
                        } else if !p.is_empty() {
                            // If there's no colon, assume the string itself is the type (e.g. from to_name())
                            final_params.push(p.to_string());
                        }
                    }
                    if let Some(arrow) = type_name.rfind("=>") {
                        let ret_part = &type_name[arrow + 2..].trim();
                        final_type = format!("function:{}", ret_part);
                    }
                }
            }
        }
        (final_type, final_params, is_variadic)
    }
}
