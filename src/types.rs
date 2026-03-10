/// Type system for the Tejx compiler, mirroring C++ Type.h

// TypeKind enum removed as it was unused

#[derive(Debug, Clone, PartialEq)]
pub enum TejxType {
    Int16,
    Int32, // Default "int"
    Int64,
    Int128,
    Float16,
    Float32, // Default "float"
    Float64,
    Bool,
    String,

    Char,                         // 4-byte
    Class(String, Vec<TejxType>), // Name, Generics
    FixedArray(Box<TejxType>, usize),
    DynamicArray(Box<TejxType>),
    Slice(Box<TejxType>),
    Void,
    Function(Vec<TejxType>, Box<TejxType>), // (Params, Return)
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
            TejxType::Function(_, _) => false,
            _ => false,
        }
    }

    pub fn is_float(&self) -> bool {
        match self {
            TejxType::Float16 | TejxType::Float32 | TejxType::Float64 => true,
            TejxType::Function(_, _) => false,
            _ => false,
        }
    }

    pub fn is_array(&self) -> bool {
        match self {
            TejxType::FixedArray(_, _) | TejxType::DynamicArray(_) | TejxType::Slice(_) => true,
            TejxType::Class(name, _) => {
                (name.ends_with("[]") || (name.contains('[') && name.ends_with(']')))
                    && !name.starts_with("Array<")
            }
            TejxType::Function(_, _) => false,
            _ => false,
        }
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
            TejxType::Class(name, generics) if name == "Array" || name.starts_with("Array<") => {
                // If it's Array<T>, return T. If it's missing generics, return Void to fail.
                if let Some(t) = generics.first() {
                    t.clone()
                } else if name.starts_with("Array<") {
                    let inner = &name[6..name.len() - 1];
                    TejxType::from_name(inner)
                } else {
                    TejxType::Void
                }
            }
            TejxType::Class(name, _) if name.ends_with("[]") => {
                let inner = &name[0..name.len() - 2];
                // Manually map built-in types inside array brackets
                match inner {
                    "string" => TejxType::String,
                    "int" => TejxType::Int32,
                    "float" => TejxType::Float64,
                    "bool" | "boolean" => TejxType::Bool,
                    _ => TejxType::from_name(inner),
                }
            }
            TejxType::Class(name, _) if name == "ByteArray" => TejxType::Bool,
            TejxType::Class(name, generics) if name == "Array" => {
                // Return Array as-is; it should fail validation later if missing generics
                TejxType::Class(name.to_string(), generics.clone())
            }
            TejxType::String => TejxType::String,
            TejxType::Function(_, _) => TejxType::Void,
            _ => TejxType::Void,
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
            | TejxType::DynamicArray(_)
            | TejxType::Function(_, _) => 8, // Pointers/Boxed/Borrows/Function pointers
            TejxType::Slice(_) => 16, // Fat pointer: {ptr, len}
            TejxType::FixedArray(inner, count) => inner.size() * count,
            TejxType::Void => 0,
        }
    }

    pub fn from_name(name: &str) -> TejxType {
        let name = name.trim();

        if name.contains('|') {
            // Simple union handling: T | None -> T (nullable)
            // We split by |, verify if one parts match "None"
            let parts: Vec<&str> = name.split('|').map(|s| s.trim()).collect();
            for part in parts {
                if part != "None" {
                    return TejxType::from_name(part);
                }
            }
            return TejxType::Void;
        }

        if name.ends_with("]") {
            // Handle type[size]
            if let Some(open) = name.find('[') {
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

        match name {
            "int" | "int32" => TejxType::Int32,
            "int16" => TejxType::Int16,
            "int64" => TejxType::Int64,
            "int128" => TejxType::Int128,
            "float" | "float32" => TejxType::Float32,
            "float64" => TejxType::Float64,
            "float16" => TejxType::Float16,
            "string" => TejxType::String,
            "char" => TejxType::Char,
            "boolean" | "bool" => TejxType::Bool,
            "" => TejxType::Void,
            other => {
                // Support parsing generic syntax like Map<String, Int>
                if let Some(open) = other.find('<') {
                    if other.ends_with('>') {
                        let base_name = &other[..open];
                        let inner_args_str = &other[open + 1..other.len() - 1];

                        // Parse comma separated generics
                        // Note: This simple split fails for Map<String, Array<Int>>.
                        // But since TejX relies on spaces or parsing earlier, we'll implement a basic nested parsing.
                        let mut generics = Vec::new();
                        let mut current_arg = String::new();
                        let mut bracket_depth = 0;
                        for c in inner_args_str.chars() {
                            if c == '<' {
                                bracket_depth += 1;
                            } else if c == '>' {
                                bracket_depth -= 1;
                            }

                            if c == ',' && bracket_depth == 0 {
                                generics.push(TejxType::from_name(current_arg.trim()));
                                current_arg.clear();
                            } else {
                                current_arg.push(c);
                            }
                        }
                        if !current_arg.trim().is_empty() {
                            generics.push(TejxType::from_name(current_arg.trim()));
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
            TejxType::Float32 => "float".to_string(),
            TejxType::Float64 => "float64".to_string(),
            TejxType::Bool => "boolean".to_string(),
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
            TejxType::FixedArray(inner, size) => format!("{}[{}]", inner.to_name(), size),
            TejxType::DynamicArray(inner) => format!("{}[]", inner.to_name()),
            TejxType::Slice(inner) => format!("slice<{}>", inner.to_name()),
            TejxType::Void => "void".to_string(),
            TejxType::Function(params, ret) => {
                let p_names: Vec<String> = params.iter().map(|p| p.to_name()).collect();
                format!("({}) => {}", p_names.join(", "), ret.to_name())
            }
        }
    }
}
