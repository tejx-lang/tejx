use super::*;
use crate::ast::*;
use crate::token::TokenType;

impl TypeChecker {
    pub(crate) fn is_assignable(&self, target: &TejxType, value: &TejxType) -> bool {
        if target == &TejxType::Any || value == &TejxType::Any {
            return true; // prevent cascading errors
        }
        self.are_types_compatible(target, value)
    }

    pub(crate) fn is_valid_type(&self, type_name: &TejxType) -> bool {
        match type_name {
            TejxType::Int16 | TejxType::Int32 | TejxType::Int64 | TejxType::Int128 |
            TejxType::Float16 | TejxType::Float32 | TejxType::Float64 |
            TejxType::Bool | TejxType::String | TejxType::Char |
            TejxType::Void | TejxType::Any => true,
            
            TejxType::Union(types) => types.iter().all(|t| self.is_valid_type(t)),
            TejxType::Class(name, generics) if generics.is_empty() => {
                let name_str = name.as_str();
                if name_str == "Option" || name_str == "Promise" || name_str == "Ref" || name_str == "Weak" || name_str == "Array" || name_str == "Map" || name_str == "Dict" || name_str == "Pair" || name_str == "Result" {
                    return false; // These built-ins always require generics
                }
                name_str == "function" || name_str == "Iterator" || name_str == "Iterable" || name_str == "None" || name_str.contains("=>") || name_str.starts_with("function:") || self.lookup(name).is_some()
            },
            TejxType::Class(name, generics) => {
                let name_str = name.as_str();
                if name_str == "Option" || name_str == "Promise" || name_str == "Ref" || name_str == "Weak" || name_str == "Array" {
                    if generics.len() != 1 { return false; }
                } else if name_str == "Map" || name_str == "Dict" || name_str == "Pair" || name_str == "Result" {
                    if generics.len() != 2 { return false; }
                } else if let Some(sym) = self.lookup(name) {
                    if sym.generic_params.len() != generics.len() && sym.generic_params.len() > 0 {
                        // We enforce generic counts only if the class actually declared some.
                        return false;
                    }
                    if !sym.generic_params.is_empty() && !generics.is_empty() {
                        for (gp, concrete) in sym.generic_params.iter().zip(generics.iter()) {
                            if let Some(bound) = &gp.bound {
                                let bound_ty = TejxType::from_node(&bound);
                                // We cannot do full `is_assignable` if it requires &mut self, but wait!
                                // `is_assignable` takes `&self`!
                                if !self.is_assignable(&bound_ty, concrete) {
                                    return false;
                                }
                            }
                        }
                    }
                }
                generics.iter().all(|g| self.is_valid_type(g))
            },
            TejxType::FixedArray(inner, _) | TejxType::DynamicArray(inner) | TejxType::Slice(inner) => self.is_valid_type(inner),
            TejxType::Function(params, ret) => {
                params.iter().all(|p| self.is_valid_type(p)) && self.is_valid_type(ret)
            }
            TejxType::Object(props) => {
                props.iter().all(|(_, _, t)| self.is_valid_type(t))
            }
        }
    }

    pub(crate) fn is_numeric(&self, t: &TejxType) -> bool {
        t.is_numeric()
    }

    pub(crate) fn get_common_ancestor(&self, t1: &TejxType, t2: &TejxType) -> TejxType {
        if t1 == t2 {
            return t1.clone();
        }
        
        if self.is_numeric(t1) && self.is_numeric(t2) {
            // Default to int32 for mixed ints, or generic logic
            if !t1.is_float() && !t2.is_float() {
                return TejxType::Int32;
            }
        }
        
        let t1_name = t1.to_name();
        let t2_name = t2.to_name();
        if t1_name == "<inferred>" { return t2.clone(); }
        if t2_name == "<inferred>" { return t1.clone(); }
        
        let mut t1_ancestors = std::collections::HashSet::new();
        let mut curr = t1_name.clone();
        t1_ancestors.insert(curr.clone());
        while let Some(parent) = self.class_hierarchy.get(&curr) {
            t1_ancestors.insert(parent.clone());
            curr = parent.clone();
        }

        curr = t2_name.clone();
        if t1_ancestors.contains(&curr) {
            return TejxType::from_name(&curr);
        }
        while let Some(parent) = self.class_hierarchy.get(&curr) {
            if t1_ancestors.contains(parent) {
                return TejxType::from_name(parent);
            }
            curr = parent.clone();
        }
        TejxType::from_name("<inferred>")
    }

    pub(crate) fn are_types_compatible(&self, expected: &TejxType, actual: &TejxType) -> bool {
        if expected == actual {
            return true;
        }
        if expected == &TejxType::Any || actual == &TejxType::Any {
            return true;
        }

        // Fast string fallback for "<inferred>"
        let e_name = expected.to_name();
        let a_name = actual.to_name();
        if e_name == "<inferred>" || a_name == "<inferred>" {
            return true;
        }

        if let TejxType::Union(types) = actual {
            if types.iter().any(|p| self.are_types_compatible(expected, p)) {
                return true;
            }
        }

        if let TejxType::Union(types) = expected {
            if types.iter().any(|p| self.are_types_compatible(p, actual)) {
                return true;
            }
        }

        // Generic wildcard: single uppercase letter mapped via Class
        let is_generic_wildcard = |t: &TejxType| -> bool {
            if let TejxType::Class(name, gen) = t {
                if gen.is_empty() && name.len() <= 2 && name.chars().next().map_or(false, |c| c.is_uppercase()) {
                    return true;
                }
            }
            false
        };
        if is_generic_wildcard(expected) || is_generic_wildcard(actual) {
            return true;
        }

        // Option<T> check
        if let TejxType::Class(name, gen) = expected {
            if name == "Option" && gen.len() == 1 {
                if a_name == "None" || self.are_types_compatible(&gen[0], actual) {
                    return true;
                }
            }
        }
        if let TejxType::Class(name, gen) = actual {
            if name == "Option" && gen.len() == 1 {
                if e_name == "None" || self.are_types_compatible(expected, &gen[0]) {
                    return true;
                }
            }
        }

        // Resolve aliases
        let resolve_type = |t: &TejxType| -> TejxType {
            let mut current = t.clone();
            let mut loops = 0;
            while loops < 10 {
                let mut changed = false;
                if let TejxType::Class(name, _) = &current {
                    if let Some(sym) = self.lookup(name) {
                        if let Some(aliased) = &sym.aliased_type {
                            current = aliased.clone();
                            changed = true;
                        }
                    }
                }
                if !changed { break; }
                loops += 1;
            }
            current
        };

        let resolved_expected = resolve_type(expected);
        let resolved_actual = resolve_type(actual);
        
        if resolved_expected != *expected || resolved_actual != *actual {
            if resolved_expected == resolved_actual { return true; }
            // Let's re-eval with resolved if they are different
            // But to avoid recursion, we just check structurally below on the resolved types
        }
        
        let expected = &resolved_expected;
        let actual = &resolved_actual;

        if e_name == "Node" {
            println!("DEBUG NODE CHECK: expr expected={:?}, resolved expected={:?}, actual={:?}", expected, resolved_expected, actual);
        }

        if let TejxType::Class(name, _) = expected {
            if name.starts_with("$MISSING_GENERIC_") { return true; }
        }
        if let TejxType::Class(name, _) = actual {
            if name.starts_with("$MISSING_GENERIC_") { return true; }
        }

        // char is compatible with string
        if (expected == &TejxType::String && actual == &TejxType::Char) || 
           (expected == &TejxType::Char && actual == &TejxType::String) {
            return true;
        }

        if expected.is_numeric() && actual.is_numeric() {
            // TejX allows wide implicit casts across numerics
            return true;
        }

        if actual.to_name() == "None" {
            let e_name = expected.to_name();
            if e_name.contains("| None") || e_name.starts_with("Option<") {
                return true;
            }
        }

        // Maps and Objects
        if let TejxType::Class(name, _) = expected {
            if name == "Map" && actual.to_name().starts_with("{") {
                return true;
            }
        }

        // Array compat
        match (expected, actual) {
            (TejxType::DynamicArray(e_in), TejxType::DynamicArray(a_in)) => {
                if self.are_types_compatible(e_in, a_in) { return true; }
            }
            (TejxType::DynamicArray(e_in), TejxType::FixedArray(a_in, _)) => {
                if self.are_types_compatible(e_in, a_in) { return true; }
            }
            (TejxType::FixedArray(e_in, _), TejxType::DynamicArray(a_in)) => {
                if self.are_types_compatible(e_in, a_in) { return true; }
            }
            (TejxType::FixedArray(e_in, e_n), TejxType::FixedArray(a_in, a_n)) => {
                if e_n == a_n && self.are_types_compatible(e_in, a_in) { return true; }
            }
            (TejxType::Slice(e_in), TejxType::Slice(a_in)) => {
                if self.are_types_compatible(e_in, a_in) { return true; }
            }
            (TejxType::Slice(e_in), TejxType::DynamicArray(a_in)) |
            (TejxType::Slice(e_in), TejxType::FixedArray(a_in, _)) => {
                if self.are_types_compatible(e_in, a_in) { return true; }
            }
            (TejxType::Slice(e_in), TejxType::String) => {
                if e_in.as_ref() == &TejxType::Char || e_in.as_ref() == &TejxType::Int32 { return true; }
            }
            (TejxType::Class(name, e_gen), _) if name == "Array" && e_gen.len() == 1 => {
                match actual {
                    TejxType::DynamicArray(a_in) | TejxType::FixedArray(a_in, _) | TejxType::Slice(a_in) => {
                        if self.are_types_compatible(&e_gen[0], a_in) { return true; }
                    }
                    TejxType::Class(a_name, a_gen) if a_name == "Array" && a_gen.len() == 1 => {
                        if self.are_types_compatible(&e_gen[0], &a_gen[0]) { return true; }
                    }
                    _ => {}
                }
            }
            (_, TejxType::Class(name, a_gen)) if name == "Array" && a_gen.len() == 1 => {
                match expected {
                    TejxType::DynamicArray(e_in) | TejxType::FixedArray(e_in, _) | TejxType::Slice(e_in) => {
                        if self.are_types_compatible(e_in, &a_gen[0]) { return true; }
                    }
                    _ => {}
                }
            }
            (TejxType::DynamicArray(e_in), TejxType::Class(n, _)) => {
                if n.ends_with("[]") {
                    let a_inner = TejxType::from_name(&n[0..n.len() - 2]);
                    if self.are_types_compatible(e_in, &a_inner) { return true; }
                } else if n.ends_with(']') && n.contains('[') {
                    let base = &n[..n.rfind('[').unwrap()];
                    let a_inner = TejxType::from_name(base);
                    if self.are_types_compatible(e_in, &a_inner) { return true; }
                }
            }
            (TejxType::Class(n, _), TejxType::DynamicArray(a_in)) => {
                if n.ends_with("[]") {
                    let e_inner = TejxType::from_name(&n[0..n.len() - 2]);
                    if self.are_types_compatible(&e_inner, a_in) { return true; }
                } else if n.ends_with(']') && n.contains('[') {
                    let base = &n[..n.rfind('[').unwrap()];
                    let e_inner = TejxType::from_name(base);
                    if self.are_types_compatible(&e_inner, a_in) { return true; }
                }
            }
            _ => {}
        }
        
        let a_str = actual.to_name();
        let e_str = expected.to_name();

        if let TejxType::Class(n, g) = actual {
            if n == "[]" && g.is_empty() {
                if matches!(expected, TejxType::DynamicArray(_) | TejxType::FixedArray(_, _) | TejxType::Slice(_)) {
                    return true;
                }
                if let TejxType::Class(en, _) = expected { if en == "Array" { return true; } }
            }
        }

        // Inheritance check
        if let Some(parent) = self.class_hierarchy.get(&a_str) {
            if self.are_types_compatible(expected, &TejxType::from_name(parent)) {
                return true;
            }
        }

        // Interface check
        if let Some(interfaces) = self.interfaces.get(&e_str) {
            if let Some(actual_members) = self.class_members.get(&a_str) {
                let mut all_match = true;
                for method_name in interfaces.keys() {
                    if !actual_members.contains_key(method_name) {
                        all_match = false;
                        break;
                    }
                }
                if all_match { return true; }
            }
        }

        // Function type compatibility
        if let TejxType::Function(e_params, e_ret) = expected {
            if let TejxType::Class(name, _) = actual {
                if name == "function" {
                    return true;
                }
            }
            
            if let TejxType::Function(a_params, a_ret) = actual {
                let ret_ok = if is_generic_wildcard(e_ret) && a_ret.as_ref() == &TejxType::Void {
                    true
                } else if is_generic_wildcard(a_ret) && e_ret.as_ref() == &TejxType::Void {
                    true
                } else {
                    self.are_types_compatible(e_ret, a_ret)
                };

                if e_str.contains("=> U") {
                    println!("DEBUG fcompat: expected={}, actual={}, ret_ok={}, e_params={:?}, a_params={:?}", e_str, a_str, ret_ok, e_params, a_params);
                }

                if ret_ok {
                    if e_params.len() == a_params.len() {
                        let mut all_params_ok = true;
                        for (ep, ap) in e_params.iter().zip(a_params.iter()) {
                            if !self.are_types_compatible(ep, ap) && ep != &TejxType::Any && ap != &TejxType::Any {
                                all_params_ok = false;
                                break;
                            }
                        }
                        if all_params_ok {
                            return true;
                        }
                    }
                }
            }
        }
        
        if let TejxType::Class(name, _) = expected {
            if name == "function" {
                if let TejxType::Function(_, _) = actual {
                    return true;
                }
            }
        }

        // Structural Object Type check
        if let TejxType::Object(e_props) = expected {
            if let TejxType::Object(a_props) = actual {
                for (a_key, _, _) in a_props {
                    if !e_props.iter().any(|(e_k, _, _)| e_k == a_key) {
                        return false;
                    }
                }

                for (e_key, e_opt, e_type) in e_props {
                    let mut found = false;
                    for (a_key, _, a_type) in a_props {
                        if e_key == a_key {
                            found = true;
                            if !self.are_types_compatible(e_type, a_type) {
                                return false;
                            }
                            break;
                        }
                    }
                    if !found && !*e_opt {
                        return false;
                    }
                }
                return true;
            }
        }

        if let TejxType::Class(e_name, _) = expected {
            if let TejxType::Object(a_props) = actual {
                let is_inline_struct = e_name.starts_with('{') && e_name.ends_with('}');
                
                if is_inline_struct {
                    println!("DEBUG compat inline struct: starting check... e_name: {}", e_name);
                    // Extract "value: int; next: Node | None" from "{ value: int; next: Node | None }"
                    let inner = e_name[1..e_name.len() - 1].trim();
                    let mut expected_props = Vec::new();
                    for part in inner.split(';') {
                        let p = part.trim();
                        if p.is_empty() { continue; }
                        if let Some(colon) = p.find(':') {
                            let mut k = p[..colon].trim().to_string();
                            let is_opt = if k.ends_with('?') {
                                k.pop();
                                true
                            } else {
                                false
                            };
                            let mut ty_str = p[colon + 1..].trim().to_string();
                            if is_opt && !ty_str.starts_with("Option<") {
                                ty_str = format!("Option<{}>", ty_str);
                            }
                            expected_props.push((k, ty_str));
                        }
                    }
                    for (a_key, _, _) in a_props {
                        if !expected_props.iter().any(|(e_k, _)| e_k == a_key) {
                            println!("DEBUG SILENT FAILURE: Property {} not in expected {:?}", a_key, expected_props);
                            return false;
                        }
                    }
                    for (e_key, e_ty_str) in expected_props {
                        let is_optional = e_ty_str.starts_with("Option<");
                        let mut found = false;
                        for (a_key, _, a_type) in a_props {
                            if e_key == *a_key {
                                found = true;
                                let mut exp_ty = TejxType::from_name(&e_ty_str);
                                let mut act_ty = a_type.clone();
                                if let TejxType::Class(n, g) = &exp_ty {
                                    if n == "Option" && g.len() == 1 { exp_ty = g[0].clone(); }
                                }
                                if let TejxType::Class(n, g) = a_type {
                                    if n == "Option" && g.len() == 1 { act_ty = g[0].clone(); }
                                }
                                let is_compat = self.are_types_compatible(&exp_ty, &act_ty);
                                if !is_compat {
                                    println!("DEBUG compat inline struct: key '{}' failed match. exp_ty: {:?}, act_ty: {:?}", e_key, exp_ty, act_ty);
                                    return false;
                                }
                                break;
                            }
                        }
                        if !found && !is_optional {
                            println!("DEBUG compat inline struct: key '{}' missing", e_key);
                            return false;
                        }
                    }
                    println!("DEBUG compat success!");
                    return true;
                } else if let Some(expected_members) = self.class_members.get(e_name) {
                    for (a_key, _, _) in a_props {
                        if !expected_members.contains_key(a_key) {
                            return false;
                        }
                    }
                    for (e_key, e_info) in expected_members {
                        let is_optional = e_info.ty.to_name().starts_with("Option<");
                        let mut found = false;
                        for (a_key, _, a_type) in a_props {
                            if e_key == a_key {
                                found = true;
                                if !self.are_types_compatible(&e_info.ty, a_type) {
                                    return false;
                                }
                                break;
                            }
                        }
                        let is_method = e_info.ty.to_name().starts_with("function:") || matches!(e_info.ty, TejxType::Function(_, _));
                        if !found && !is_optional && !is_method {
                            return false;
                        }
                    }
                    return true;
                }
            }
        }

        false
    }

    pub(crate) fn strip_none_from_union(&self, type_name: &TejxType) -> TejxType {
        if let TejxType::Union(types) = type_name {
            let mut filtered = Vec::new();
            for t in types {
                if t.to_name() != "None" {
                    filtered.push(t.clone());
                }
            }
            if filtered.len() == 1 {
                return filtered[0].clone();
            } else if filtered.is_empty() {
                return TejxType::Void;
            } else {
                return TejxType::Union(filtered);
            }
        }
        type_name.clone()
    }

    pub(crate) fn check_numeric_bounds(
        &mut self,
        expr: &Expression,
        target_type: &TejxType,
        line: usize,
        col: usize,
    ) {
        let mut val_to_check = None;
        if let Expression::NumberLiteral { value, .. } = expr {
            val_to_check = Some(*value);
        } else if let Expression::UnaryExpr { op: TokenType::Minus, right, .. } = expr {
            if let Expression::NumberLiteral { value, .. } = &**right {
                val_to_check = Some(-*value);
            }
        }

        if let Some(v) = val_to_check {
            let mut min = None;
            let mut max = None;
            let t_name = target_type.to_name();
            match t_name.as_str() {
                "int8" => { min = Some(-128.0); max = Some(127.0); }
                "uint8" => { min = Some(0.0); max = Some(255.0); }
                "int16" => { min = Some(-32768.0); max = Some(32767.0); }
                "uint16" => { min = Some(0.0); max = Some(65535.0); }
                _ => {}
            }
            
            if min.is_some() && max.is_some() {
                if v < min.unwrap() || v > max.unwrap() {
                    self.report_error_detailed(
                        format!("Numeric literal '{}' is out of bounds for type '{}'", v, t_name),
                        line, col, "E0111", Some(&format!("Ensure the value is between {} and {}", min.unwrap(), max.unwrap()))
                    );
                }
            }
        }
    }
}
