use super::*;

impl TypeChecker {
    pub fn register_instantiation(&mut self, type_name: &str, line: usize, col: usize) {
        if let Some(open) = type_name.find('<') {
            if type_name.ends_with('>') {
                let base = &type_name[..open];
                let inner = &type_name[open + 1..type_name.len() - 1];

                let mut args = Vec::new();
                let mut depth_level = 0;
                let mut start = 0;
                for (i, c) in inner.char_indices() {
                    match c {
                        '<' => depth_level += 1,
                        '>' => depth_level -= 1,
                        ',' if depth_level == 0 => {
                            let arg_ty = inner[start..i].trim();
                            args.push(TejxType::from_name(arg_ty));
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                args.push(TejxType::from_name(inner[start..].trim()));

                self.generic_instantiations
                    .entry(base.to_string())
                    .or_default()
                    .insert(args.clone());

                if let Some(s) = self.lookup(base) {
                    if s.generic_params.len() == args.len() {
                        for (i, gp) in s.generic_params.iter().enumerate() {
                            if let Some(bound) = &gp.bound {
                                let bound_str = bound.to_string();
                                let concrete = &args[i].to_name();
                                if !self.is_assignable(&TejxType::from_name(&bound_str), &TejxType::from_name(concrete)) {
                                    self.report_error_detailed(
                                        format!("Type '{}' does not satisfy constraint '{}' for generic parameter '{}'", concrete, bound_str, gp.name),
                                        line,
                                        col,
                                        "E0120",
                                        Some(&format!("Provide a type that satisfies the constraint '{}'", bound_str))
                                    );
                                }
                            }
                        }
                    }
                }

                for arg in args {
                    self.register_instantiation(&arg.to_name(), line, col);
                }
            }
        } else if type_name.ends_with("[]") {
            let base = &type_name[..type_name.len() - 2];
            self.register_instantiation(base, line, col);
        }
    }

    pub(crate) fn substitute_generics(&self, member_type: &str, obj_type: &str) -> String {
        let mut parts = Vec::new();
        if let Some(open) = obj_type.find('<') {
            if let Some(close) = obj_type.rfind('>') {
                let inner = &obj_type[open + 1..close];
                // Split inner by comma, respecting nested generics and object literals
                let mut start = 0;
                let mut depth = 0;
                let mut depth_brace = 0;
                for (i, c) in inner.char_indices() {
                    match c {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        '{' => depth_brace += 1,
                        '}' => depth_brace -= 1,
                        ',' if depth == 0 && depth_brace == 0 => {
                            parts.push(inner[start..i].trim());
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                parts.push(inner[start..].trim());
            }
        } else if obj_type.ends_with("[]") {
            parts.push(&obj_type[..obj_type.len() - 2]);
        }

        let mut result = member_type.to_string();
        // Check for $0, $1... up to some reasonable limit or until no more are found
        for i in 0..5 {
            let placeholder = format!("${}", i);
            if result.contains(&placeholder) {
                let replacement = if i < parts.len() {
                    parts[i].to_string()
                } else {
                    format!("$MISSING_GENERIC_{}", i)
                };
                result = result.replace(&placeholder, &replacement);
            }
        }

        result
    }

    pub(crate) fn parameterize_generics(
        &self,
        type_name: &str,
        params: &Vec<crate::ast::GenericParam>,
    ) -> String {
        let mut result = type_name.to_string();
        for (i, param) in params.iter().enumerate() {
            let placeholder = format!("${}", i);
            let mut new_res = String::new();
            let mut last_pos = 0;
            let p_len = param.name.len();

            while let Some(idx) = result[last_pos..].find(&param.name) {
                let abs_idx = last_pos + idx;
                // Fix indexing: operate on byte slices
                let before_char = if abs_idx > 0 {
                    result[..abs_idx].chars().last()
                } else {
                    None
                };
                let after_char = result[abs_idx + p_len..].chars().next();

                let is_word_start = match before_char {
                    Some(c) => !c.is_alphanumeric() && c != '_',
                    None => true,
                };
                let is_word_end = match after_char {
                    Some(c) => !c.is_alphanumeric() && c != '_',
                    None => true,
                };

                new_res.push_str(&result[last_pos..abs_idx]);

                if is_word_start && is_word_end {
                    new_res.push_str(&placeholder);
                } else {
                    new_res.push_str(&param.name);
                }
                last_pos = abs_idx + p_len;
            }
            new_res.push_str(&result[last_pos..]);
            result = new_res;
        }
        result
    }
}
