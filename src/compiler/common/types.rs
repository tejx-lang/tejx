/// Type system for the Tejx compiler, mirroring C++ Type.h

// TypeKind enum removed as it was unused

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TejxType {
    Int16,
    Int32, // Default "int"
    Int64,
    Int128,
    Float16,
    Float32,
    Float64,
    Bool,
    String,

    Char,                         // 4-byte
    Class(String, Vec<TejxType>), // Name, Generics
    Optional(Box<TejxType>),
    FixedArray(Box<TejxType>, usize),
    DynamicArray(Box<TejxType>),
    Slice(Box<TejxType>),
    Void,
    Function(Vec<TejxType>, Box<TejxType>), // (Params, Return)
    Object(Vec<(String, bool, TejxType)>),
    Any,
}

pub(crate) fn split_top_level(input: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth_angle = 0usize;
    let mut depth_brace = 0usize;
    let mut depth_bracket = 0usize;
    let mut depth_paren = 0usize;

    for (i, ch) in input.char_indices() {
        match ch {
            '<' => depth_angle += 1,
            '>' => {
                depth_angle = depth_angle.saturating_sub(1);
            }
            '{' => depth_brace += 1,
            '}' => {
                depth_brace = depth_brace.saturating_sub(1);
            }
            '[' => depth_bracket += 1,
            ']' => {
                depth_bracket = depth_bracket.saturating_sub(1);
            }
            '(' => depth_paren += 1,
            ')' => {
                depth_paren = depth_paren.saturating_sub(1);
            }
            _ => {}
        }

        if ch == sep && depth_angle == 0 && depth_brace == 0 && depth_bracket == 0 && depth_paren == 0 {
            parts.push(input[start..i].trim());
            start = i + ch.len_utf8();
        }
    }

    if start <= input.len() {
        parts.push(input[start..].trim());
    }

    parts
}

pub(crate) fn find_top_level_generic_bounds(input: &str) -> Option<(usize, usize)> {
    let mut depth_angle = 0usize;
    let mut depth_brace = 0usize;
    let mut depth_bracket = 0usize;
    let mut depth_paren = 0usize;
    let mut open = None;

    for (i, ch) in input.char_indices() {
        match ch {
            '{' => depth_brace += 1,
            '}' => {
                depth_brace = depth_brace.saturating_sub(1);
            }
            '[' => depth_bracket += 1,
            ']' => {
                depth_bracket = depth_bracket.saturating_sub(1);
            }
            '(' => depth_paren += 1,
            ')' => {
                depth_paren = depth_paren.saturating_sub(1);
            }
            '<' => {
                if depth_brace == 0 && depth_bracket == 0 && depth_paren == 0 && depth_angle == 0 {
                    open = Some(i);
                }
                depth_angle += 1;
            }
            '>' => {
                if depth_angle > 0 {
                    depth_angle -= 1;
                    if depth_angle == 0 {
                        if let Some(open_idx) = open {
                            return Some((open_idx, i));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    None
}

impl TejxType {
    pub fn is_numeric(&self) -> bool {
        match self {
            TejxType::Int16
            | TejxType::Int32
            | TejxType::Int64
            | TejxType::Int128
            | TejxType::Float16
            | TejxType::Float32
            | TejxType::Float64 => true,
            TejxType::Function(_, _) | TejxType::Optional(_) | TejxType::Object(_) => false,
            _ => false,
        }
    }

    pub fn is_float(&self) -> bool {
        match self {
            TejxType::Float16 | TejxType::Float32 | TejxType::Float64 => true,
            TejxType::Function(_, _) | TejxType::Optional(_) | TejxType::Object(_) => false,
            _ => false,
        }
    }

    pub fn is_array(&self) -> bool {
        matches!(
            self,
            TejxType::FixedArray(_, _) | TejxType::DynamicArray(_) | TejxType::Slice(_)
        )
    }

    pub fn is_object(&self) -> bool {
        matches!(self, TejxType::Class(_, _) | TejxType::Object(_) | TejxType::Any)
    }

    pub fn is_slice(&self) -> bool {
        match self {
            TejxType::Slice(_) => true,
            TejxType::Function(_, _) => false,
            _ => false,
        }
    }

    pub fn get_array_element_type(&self) -> TejxType {
        match self {
            TejxType::FixedArray(inner, _)
            | TejxType::DynamicArray(inner)
            | TejxType::Slice(inner) => (**inner).clone(),
            TejxType::Function(_, _) => TejxType::Void,
            _ => TejxType::Any,
        }

    }

    pub fn size(&self) -> usize {
        match self {
            TejxType::Int16 | TejxType::Float16 => 2,
            TejxType::Int32 | TejxType::Float32 => 4,
            TejxType::Int64 | TejxType::Float64 => 8,
            TejxType::Int128 => 16,
            TejxType::Bool => 1,
            TejxType::Char => 4,
            TejxType::String
            | TejxType::Class(_, _)
            | TejxType::Optional(_)
            | TejxType::DynamicArray(_)
            | TejxType::Object(_) => 8, // Pointers/Boxed/Borrows
            TejxType::Function(_, _) => 16, // Pointer + env (fat pointer)
            TejxType::Slice(_) => 16, // Fat pointer: {ptr, len}
            TejxType::FixedArray(inner, count) => inner.size() * count,
            TejxType::Any => 8, // Boxed value ptr
            TejxType::Void => 0,
        }
    }

    pub fn from_node(node: &crate::frontend::ast::TypeNode) -> TejxType {
        match node {
            crate::frontend::ast::TypeNode::Named(name) => TejxType::from_name(name),
            crate::frontend::ast::TypeNode::Generic(name, args) => {
                let parsed_args = args.iter().map(TejxType::from_node).collect();
                TejxType::Class(name.clone(), parsed_args)
            }
            crate::frontend::ast::TypeNode::Optional(inner) => {
                TejxType::Optional(Box::new(TejxType::from_node(inner)))
            }
            crate::frontend::ast::TypeNode::Array(inner) => {
                TejxType::DynamicArray(Box::new(TejxType::from_node(inner)))
            }
            crate::frontend::ast::TypeNode::SizedArray(inner, size_expr) => {
                let inner_ty = TejxType::from_node(inner);
                match size_expr.as_ref() {
                    crate::frontend::ast::Expression::NumberLiteral { value, .. }
                        if *value >= 0.0 && value.fract() == 0.0 =>
                    {
                        TejxType::FixedArray(Box::new(inner_ty), *value as usize)
                    }
                    _ => TejxType::DynamicArray(Box::new(inner_ty)),
                }
            }
            crate::frontend::ast::TypeNode::Function(params, ret) => {
                let parsed_params = params.iter().map(TejxType::from_node).collect();
                TejxType::Function(parsed_params, Box::new(TejxType::from_node(ret)))
            }
            crate::frontend::ast::TypeNode::Object(members) => {
                let parsed_members = members.iter().map(|(k, o, t)| (k.clone(), *o, TejxType::from_node(t))).collect();
                TejxType::Object(parsed_members)
            }
            crate::frontend::ast::TypeNode::Intersection(_) => TejxType::Any,
            crate::frontend::ast::TypeNode::Any => TejxType::Any,
        }
    }

    pub fn to_type_node(&self) -> crate::frontend::ast::TypeNode {
        match self {
            TejxType::Int16 => crate::frontend::ast::TypeNode::Named("int16".to_string()),
            TejxType::Int32 => crate::frontend::ast::TypeNode::Named("int32".to_string()),
            TejxType::Int64 => crate::frontend::ast::TypeNode::Named("int64".to_string()),
            TejxType::Int128 => crate::frontend::ast::TypeNode::Named("int128".to_string()),
            TejxType::Float16 => crate::frontend::ast::TypeNode::Named("float16".to_string()),
            TejxType::Float32 => crate::frontend::ast::TypeNode::Named("float32".to_string()),
            TejxType::Float64 => crate::frontend::ast::TypeNode::Named("float64".to_string()),
            TejxType::Bool => crate::frontend::ast::TypeNode::Named("bool".to_string()),
            TejxType::String => crate::frontend::ast::TypeNode::Named("string".to_string()),
            TejxType::Char => crate::frontend::ast::TypeNode::Named("char".to_string()),
            TejxType::Void => crate::frontend::ast::TypeNode::Named("void".to_string()),
            TejxType::Any => crate::frontend::ast::TypeNode::Any,
            TejxType::Class(name, generics) => {
                if generics.is_empty() {
                    crate::frontend::ast::TypeNode::Named(name.clone())
                } else {
                    let type_args = generics.iter().map(|g| g.to_type_node()).collect();
                    crate::frontend::ast::TypeNode::Generic(name.clone(), type_args)
                }
            }
            TejxType::Optional(inner) => crate::frontend::ast::TypeNode::Optional(Box::new(inner.to_type_node())),
            TejxType::FixedArray(inner, size) => crate::frontend::ast::TypeNode::SizedArray(
                Box::new(inner.to_type_node()),
                Box::new(crate::frontend::ast::Expression::NumberLiteral {
                    value: *size as f64,
                    _is_float: false,
                    _line: 0,
                    _col: 0,
                }),
            ),
            TejxType::DynamicArray(inner) => crate::frontend::ast::TypeNode::Array(Box::new(inner.to_type_node())),
            TejxType::Slice(inner) => crate::frontend::ast::TypeNode::Generic("slice".to_string(), vec![inner.to_type_node()]),
            TejxType::Function(params, ret) => {
                let p_nodes = params.iter().map(|p| p.to_type_node()).collect();
                crate::frontend::ast::TypeNode::Function(p_nodes, Box::new(ret.to_type_node()))
            }
            TejxType::Object(members) => crate::frontend::ast::TypeNode::Object(members.iter().map(|(k, o, t)| (k.clone(), *o, t.to_type_node())).collect()),
        }
    }

    pub fn from_name(name: &str) -> TejxType {
        let name = name.trim();

        let union_parts = split_top_level(name, '|');
        if union_parts.len() > 1 {
            let parts: Vec<TejxType> = union_parts
                .into_iter()
                .filter(|p| !p.is_empty())
                .map(|s| TejxType::from_name(s.trim()))
                .collect();
            if parts.len() == 2 {
                let left_is_none = parts[0].to_name() == "None";
                let right_is_none = parts[1].to_name() == "None";
                if left_is_none ^ right_is_none {
                    let inner = if left_is_none {
                        parts[1].clone()
                    } else {
                        parts[0].clone()
                    };
                    return TejxType::Optional(Box::new(inner));
                }
            }
            return TejxType::Class(name.to_string(), vec![]);
        }

        if name.starts_with('{') && name.ends_with('}') {
            let inner = name[1..name.len() - 1].trim();
            let mut props = Vec::new();
            let mut current = String::new();
            let mut depth_brace = 0usize;
            let mut depth_angle = 0usize;
            let mut depth_bracket = 0usize;
            let mut depth_paren = 0usize;

            let flush_prop = |buf: &str, props: &mut Vec<(String, bool, TejxType)>| {
                let p = buf.trim();
                if p.is_empty() {
                    return;
                }
                if let Some(colon) = p.find(':') {
                    let mut key = p[..colon].trim().to_string();
                    let mut is_opt = false;
                    if key.ends_with('?') {
                        key.pop();
                        is_opt = true;
                    }
                    let ty_str = p[colon + 1..].trim();
                    let ty = TejxType::from_name(ty_str);
                    props.push((key, is_opt, ty));
                }
            };

            for ch in inner.chars() {
                match ch {
                    '{' => {
                        depth_brace += 1;
                        current.push(ch);
                    }
                    '}' => {
                        depth_brace = depth_brace.saturating_sub(1);
                        current.push(ch);
                    }
                    '<' => {
                        depth_angle += 1;
                        current.push(ch);
                    }
                    '>' => {
                        depth_angle = depth_angle.saturating_sub(1);
                        current.push(ch);
                    }
                    '[' => {
                        depth_bracket += 1;
                        current.push(ch);
                    }
                    ']' => {
                        depth_bracket = depth_bracket.saturating_sub(1);
                        current.push(ch);
                    }
                    '(' => {
                        depth_paren += 1;
                        current.push(ch);
                    }
                    ')' => {
                        depth_paren = depth_paren.saturating_sub(1);
                        current.push(ch);
                    }
                    ';' | ',' if depth_brace == 0
                        && depth_angle == 0
                        && depth_bracket == 0
                        && depth_paren == 0 =>
                    {
                        let buf = current.clone();
                        flush_prop(&buf, &mut props);
                        current.clear();
                    }
                    _ => current.push(ch),
                }
            }
            if !current.trim().is_empty() {
                let buf = current.clone();
                flush_prop(&buf, &mut props);
            }
            return TejxType::Object(props);
        }

        if name.ends_with("]") {
            // Handle type[size]
            if let Some(open) = name.rfind('[') {
                let base = &name[..open];
                let size_str = &name[open + 1..name.len() - 1];
                if size_str.is_empty() {
                    return TejxType::DynamicArray(Box::new(TejxType::from_name(base)));
                }
                if let Ok(size) = size_str.parse::<usize>() {
                    return TejxType::FixedArray(Box::new(TejxType::from_name(base)), size);
                }
                // Fallback to dynamic array? or Class?
                return TejxType::Class(name.to_string(), vec![]);
            }
        }

        if name.starts_with("slice<") && name.ends_with(">") {
            let inner = &name[6..name.len() - 1];
            return TejxType::Slice(Box::new(TejxType::from_name(inner)));
        }

        if name.starts_with("Optional<") && name.ends_with('>') {
            let inner = &name[name.find('<').unwrap_or(0) + 1..name.len() - 1];
            return TejxType::Optional(Box::new(TejxType::from_name(inner)));
        }

        match name {
            "int" | "int32" => TejxType::Int32,
            "int16" => TejxType::Int16,
            "int64" => TejxType::Int64,
            "int128" => TejxType::Int128,
            "float" => TejxType::Float32,
            "float32" => TejxType::Float32,
            "float64" => TejxType::Float64,
            "float16" => TejxType::Float16,
            "string" => TejxType::String,
            "char" => TejxType::Char,
            "bool" => TejxType::Bool,
            "void" | "" => TejxType::Void,
            "any" => TejxType::Any,
            other => {
                // Support parsing generic syntax like Map<String, Int>
                if let Some((open, close)) = find_top_level_generic_bounds(other) {
                    if close + 1 == other.len() {
                        let base_name = other[..open].trim();
                        let inner_args_str = &other[open + 1..close];
                        let mut generics = Vec::new();
                        for arg in split_top_level(inner_args_str, ',') {
                            let arg = arg.trim();
                            if !arg.is_empty() {
                                generics.push(TejxType::from_name(arg));
                            }
                        }
                        return TejxType::Class(base_name.to_string(), generics);
                    }
                }
                TejxType::Class(other.to_string(), vec![])
            }
        }
    }

    pub fn to_name(&self) -> String {
        match self {
            TejxType::Int16 => "int16".to_string(),
            TejxType::Int32 => "int".to_string(),
            TejxType::Int64 => "int64".to_string(),
            TejxType::Int128 => "int128".to_string(),
            TejxType::Float16 => "float16".to_string(),
            TejxType::Float32 => "float32".to_string(),
            TejxType::Float64 => "float64".to_string(),
            TejxType::Bool => "bool".to_string(),
            TejxType::String => "string".to_string(),
            TejxType::Char => "char".to_string(),
            TejxType::Class(name, generics) => {
                if generics.is_empty() {
                    name.clone()
                } else {
                    let gen_strs: Vec<String> = generics.iter().map(|g| g.to_name()).collect();
                    format!("{}<{}>", name, gen_strs.join(", "))
                }
            }
            TejxType::Optional(inner) => format!("Optional<{}>", inner.to_name()),
            TejxType::Any => "any".to_string(),
            TejxType::FixedArray(inner, size) => format!("{}[{}]", inner.to_name(), size),
            TejxType::DynamicArray(inner) => format!("{}[]", inner.to_name()),
            TejxType::Slice(inner) => format!("slice<{}>", inner.to_name()),
            TejxType::Void => "void".to_string(),
            TejxType::Function(params, ret) => {
                let p_names: Vec<String> = params.iter().map(|p| p.to_name()).collect();
                format!("({}) => {}", p_names.join(", "), ret.to_name())
            }
            TejxType::Object(props) => {
                let p_names: Vec<String> = props.iter().map(|(k, o, t)| {
                    let opt = if *o { "?" } else { "" };
                    format!("{}{}: {}", k, opt, t.to_name())
                }).collect();
                format!("{{ {} }}", p_names.join("; "))
            }
        }
    }

    pub fn substitute_generics(&self, bindings: &std::collections::HashMap<String, TejxType>) -> TejxType {
        match self {
            TejxType::Class(name, generics) => {
                if generics.is_empty() {
                    if let Some(sub) = bindings.get(name) {
                        return sub.clone();
                    }
                    TejxType::Class(name.clone(), vec![])
                } else {
                    let new_generics = generics.iter().map(|g| g.substitute_generics(bindings)).collect();
                    TejxType::Class(name.clone(), new_generics)
                }
            }
            TejxType::Optional(inner) => {
                TejxType::Optional(Box::new(inner.substitute_generics(bindings)))
            }
            TejxType::FixedArray(inner, size) => TejxType::FixedArray(Box::new(inner.substitute_generics(bindings)), *size),
            TejxType::DynamicArray(inner) => TejxType::DynamicArray(Box::new(inner.substitute_generics(bindings))),
            TejxType::Slice(inner) => TejxType::Slice(Box::new(inner.substitute_generics(bindings))),
            TejxType::Function(params, ret) => {
                let new_params = params.iter().map(|p| p.substitute_generics(bindings)).collect();
                TejxType::Function(new_params, Box::new(ret.substitute_generics(bindings)))
            }
            TejxType::Object(props) => TejxType::Object(
                props.iter().map(|(k, o, t)| (k.clone(), *o, t.substitute_generics(bindings))).collect()
            ),
            _ => self.clone(),
        }
    }
}
