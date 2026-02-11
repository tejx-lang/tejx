/// Type system for the Tejx compiler, mirroring C++ Type.h

// TypeKind enum removed as it was unused


#[derive(Debug, Clone, PartialEq)]
pub enum TejxType {
    Primitive(String),       // "number", "string", "boolean"
    Class(String),           // class name
    // Function variant removed (unused)
    // Array variant removed (unused)
    Void,
    Any,
}

impl TejxType {
    // to_string removed (unused)


    // unused methods removed: is_any, equals

    pub fn is_class(&self) -> bool {
        matches!(self, TejxType::Class(_))
    }


    pub fn from_name(name: &str) -> TejxType {
        match name {
            "number" | "int" | "float" => TejxType::Primitive("number".to_string()),
            "string" => TejxType::Primitive("string".to_string()),
            "boolean" | "bool" => TejxType::Primitive("boolean".to_string()),
            "void" => TejxType::Void,
            "any" | "" => TejxType::Any,
            other => TejxType::Class(other.to_string()),
        }
    }
}
