use super::Lowering;
use crate::ast::*;
use crate::hir::*;
use crate::types::TejxType;

impl Lowering {
    pub(crate) fn register_function(&self, func: &FunctionDeclaration) {
        let param_types: Vec<TejxType> = func
            .params
            .iter()
            .map(|p| TejxType::from_node(&p.type_name))
            .collect();
        let ret_type = TejxType::from_node(&func.return_type);

        let name = if func.is_extern {
            func.name.clone()
        } else {
            format!("f_{}", func.name)
        };

        if !func.generic_params.is_empty() {
            self.function_generic_params.borrow_mut().insert(
                name.clone(),
                func.generic_params
                    .iter()
                    .map(|gp| gp.name.clone())
                    .collect(),
            );
        }

        self.user_functions.borrow_mut().insert(
            name.clone(),
            TejxType::Function(param_types, Box::new(ret_type)),
        );
        self.user_function_args
            .borrow_mut()
            .insert(name, func.params.len());
        if func.is_extern {
            self.extern_functions.borrow_mut().insert(func.name.clone());
        }
    }

    pub(crate) fn substitute_generics(
        &self,
        ret_ty: &TejxType,
        obj_type: &TejxType,
        callee_name: &str,
    ) -> TejxType {
        let type_name = ret_ty.to_name();

        // Find base class name from object type or assume Array/String
        let obj_full = obj_type.to_name();
        let base_class = if obj_type.is_array() {
            "Array".to_string()
        } else if *obj_type == TejxType::String {
            "String".to_string()
        } else {
            obj_full
                .split('<')
                .next()
                .unwrap_or(&obj_full)
                .trim()
                .to_string()
        };

        // Try getting params from class or function
        let mut params = Vec::new();
        if let Some(p) = self.class_generic_params.borrow().get(&base_class) {
            params = p.clone();
        } else if let Some(p) = self.function_generic_params.borrow().get(callee_name) {
            params = p.clone();
        } else if base_class == "Array" {
            params = vec!["T".to_string()];
        }

        if params.is_empty() {
            return ret_ty.clone();
        }

        // Extract concrete type args
        let concrete_args = if obj_type.is_array() {
            vec![obj_type.get_array_element_type().to_name()]
        } else if let Some(open) = obj_full.find('<') {
            if let Some(close) = obj_full.rfind('>') {
                let inner = &obj_full[open + 1..close];
                let mut args = Vec::new();
                let mut start = 0;
                let mut depth = 0;
                for (i, c) in inner.char_indices() {
                    match c {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        ',' if depth == 0 => {
                            args.push(inner[start..i].trim().to_string());
                            start = i + 1;
                        }
                        _ => {}
                    }
                }
                args.push(inner[start..].trim().to_string());
                args
            } else {
                return ret_ty.clone();
            }
        } else {
            return ret_ty.clone();
        };

        let mut bindings = std::collections::HashMap::new();
        for (i, param) in params.iter().enumerate() {
            if i < concrete_args.len() {
                bindings.insert(param.clone(), TejxType::from_name(&concrete_args[i]));
            }
        }
        
        ret_ty.substitute_generics(&bindings)
    }

    pub(crate) fn lower_function_declaration(
        &self,
        func: &FunctionDeclaration,
        functions: &mut Vec<HIRStatement>,
    ) {
        let line = func._line;
        if self.async_enabled && func._is_async {
            self.lower_async_function(func, functions);
            return;
        }

        let params: Vec<(String, TejxType)> = func
            .params
            .iter()
            .map(|p| (p.name.clone(), TejxType::from_node(&p.type_name)))
            .collect();
        let return_type = TejxType::from_node(&func.return_type);

        self.enter_scope();
        let mangled_params: Vec<(String, TejxType)> = params
            .iter()
            .map(|(pname, pty)| (self.define(pname.clone(), pty.clone()), pty.clone()))
            .collect();

        let body = self
            .lower_statement(&func.body)
            .unwrap_or(HIRStatement::Block {
                line: line,
                statements: vec![],
            });

        self._exit_scope();

        let name = format!("f_{}", func.name);
        functions.push(HIRStatement::Function {
            async_params: None,
            line: line,
            name,
            params: mangled_params,
            _return_type: return_type,
            body: Box::new(body),
            is_extern: func.is_extern,
        });
    }
}
