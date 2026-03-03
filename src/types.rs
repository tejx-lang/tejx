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

    Char, // 4-byte
    Class(String),
    FixedArray(Box<TejxType>, usize),
    Void,
    Ref(Box<TejxType>),  // Non-owning borrow
    Weak(Box<TejxType>), // Non-owning cycle-breaker
}

impl TejxType {
    pub fn is_class(&self) -> bool {
        matches!(self, TejxType::Class(_))
    }

    pub fn needs_drop(&self) -> bool {
        match self {
            TejxType::Int16
            | TejxType::Int32
            | TejxType::Int64
            | TejxType::Int128
            | TejxType::Float16
            | TejxType::Float32
            | TejxType::Float64
            | TejxType::Bool
            | TejxType::Char
            | TejxType::Void
            | TejxType::Ref(_)
            | TejxType::Weak(_) => false,
            // Re-enabling drops. Strict checking in borrow checker will prevent double-frees.
            TejxType::String | TejxType::Class(_) | TejxType::FixedArray(_, _) => true,
        }
    }

    pub fn is_numeric(&self) -> bool {
        match self {
            TejxType::Int16
            | TejxType::Int32
            | TejxType::Int64
            | TejxType::Int128
            | TejxType::Float16
            | TejxType::Float32
            | TejxType::Float64 => true,
            TejxType::Ref(inner) | TejxType::Weak(inner) => inner.is_numeric(),
            _ => false,
        }
    }

    pub fn is_float(&self) -> bool {
        match self {
            TejxType::Float16 | TejxType::Float32 | TejxType::Float64 => true,
            TejxType::Ref(inner) | TejxType::Weak(inner) => inner.is_float(),
            _ => false,
        }
    }

    pub fn is_array(&self) -> bool {
        match self {
            TejxType::FixedArray(_, _) => true,
            TejxType::Class(name) => {
                name == "Array" || name.starts_with("Array<") || name.ends_with("[]")
            }
            TejxType::Ref(inner) | TejxType::Weak(inner) => inner.is_array(),
            _ => false,
        }
    }

    pub fn get_array_element_type(&self) -> TejxType {
        match self {
            TejxType::FixedArray(inner, _) => (**inner).clone(),
            TejxType::Class(name) if name.starts_with("Array<") => {
                // Simplified extraction: Array<T>
                let inner = &name[6..name.len() - 1];
                TejxType::from_name(inner)
            }
            TejxType::Class(name) if name.ends_with("[]") => {
                let inner = &name[0..name.len() - 2];
                TejxType::from_name(inner)
            }
            TejxType::Class(name) if name == "ByteArray" => TejxType::Bool,
            TejxType::Ref(inner) | TejxType::Weak(inner) => inner.get_array_element_type(), // Delegate to underlying type
            _ => TejxType::Void,
        }
    }

    #[allow(dead_code)]
    pub fn size(&self) -> usize {
        match self {
            TejxType::Int16 | TejxType::Float16 => 2,
            TejxType::Int32 | TejxType::Float32 => 4,
            TejxType::Int64 | TejxType::Float64 => 8,
            TejxType::Int128 => 16,
            TejxType::Bool => 1,
            TejxType::Char => 4,
            TejxType::String | TejxType::Class(_) | TejxType::Ref(_) | TejxType::Weak(_) => 8, // Pointers/Boxed/Borrows
            TejxType::FixedArray(inner, count) => inner.size() * count,
            TejxType::Void => 0,
        }
    }

    pub fn from_name(name: &str) -> TejxType {
        let name = name.trim();
        if name.starts_with("ref ") {
            return TejxType::Ref(Box::new(TejxType::from_name(&name[4..])));
        }
        if name.starts_with("weak ") {
            return TejxType::Weak(Box::new(TejxType::from_name(&name[5..])));
        }

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
                if let Ok(size) = size_str.parse::<usize>() {
                    return TejxType::FixedArray(Box::new(TejxType::from_name(base)), size);
                }
                // Fallback to dynamic array? or Class?
                return TejxType::Class(name.to_string());
            }
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
            other => TejxType::Class(other.to_string()),
        }
    }
}
