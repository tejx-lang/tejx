/// Type system for the Tejx compiler, mirroring C++ Type.h

// TypeKind enum removed as it was unused


#[derive(Debug, Clone, PartialEq)]
pub enum TejxType {
    Int16,
    Int32,   // Default "int"
    Int64,
    Int128,
    Float16,
    Float32, // Default "float"
    Float64,
    Bool,
    String,
    Char,    // 4-byte
    Class(String),
    FixedArray(Box<TejxType>, usize),
    Void,
    Any,
}

impl TejxType {
    pub fn is_class(&self) -> bool {
        matches!(self, TejxType::Class(_))
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, 
            TejxType::Int16 | TejxType::Int32 | TejxType::Int64 | TejxType::Int128 |
            TejxType::Float16 | TejxType::Float32 | TejxType::Float64
        )
    }

    pub fn is_float(&self) -> bool {
        matches!(self, TejxType::Float16 | TejxType::Float32 | TejxType::Float64)
    }

    pub fn is_array(&self) -> bool {
        matches!(self, TejxType::FixedArray(_, _)) || if let TejxType::Class(name) = self { name == "Array" || name.starts_with("Array<") || name.ends_with("[]") } else { false }
    }

    pub fn get_array_element_type(&self) -> TejxType {
        match self {
            TejxType::FixedArray(inner, _) => (**inner).clone(),
            TejxType::Class(name) if name.starts_with("Array<") => {
                // Simplified extraction: Array<T>
                let inner = &name[6..name.len()-1];
                TejxType::from_name(inner)
            }
            TejxType::Class(name) if name.ends_with("[]") => {
                let inner = &name[0..name.len()-2];
                TejxType::from_name(inner)
            }
            TejxType::Class(name) if name == "ByteArray" => TejxType::Bool,
            _ => TejxType::Any
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
            TejxType::String | TejxType::Class(_) | TejxType::Any => 8, // Pointers/Boxed
            TejxType::FixedArray(inner, count) => inner.size() * count,
            TejxType::Void => 0,
        }
    }

    pub fn from_name(name: &str) -> TejxType {
        if name.ends_with("]") {
             // Handle type[size]
             if let Some(open) = name.find('[') {
                  let base = &name[..open];
                  let size_str = &name[open+1..name.len()-1];
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
            "boolean" | "bool" => TejxType::Bool,
            "any" | "" => TejxType::Any,
            other => TejxType::Class(other.to_string()),
        }
    }
}
