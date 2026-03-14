use crate::types::TejxType;

#[derive(Clone, Copy)]
enum BuiltinRet {
    Int32,
    Bool,
    String,
    Void,
    Elem,
    SelfArray,
    StringArray,
}

struct BuiltinMethodInfo {
    callee: &'static str,
    ret: Option<BuiltinRet>,
}

const STRING_METHOD_NAMES: &[&str] = &[
    "length",
    "concat",
    "includes",
    "startsWith",
    "endsWith",
    "indexOf",
    "toUpperCase",
    "toLowerCase",
    "trim",
    "trimStart",
    "trimEnd",
    "substring",
    "split",
    "repeat",
    "replace",
    "padStart",
    "padEnd",
];

const ARRAY_METHOD_NAMES: &[&str] = &[
    "length",
    "push",
    "pop",
    "shift",
    "unshift",
    "indexOf",
    "concat",
    "join",
    "slice",
    "reverse",
    "fill",
    "sort",
    "map",
    "filter",
    "forEach",
    "reduce",
    "every",
    "some",
    "find",
    "findIndex",
    "includes",
];

fn string_method_info(name: &str) -> Option<BuiltinMethodInfo> {
    let info = match name {
        "length" => BuiltinMethodInfo {
            callee: "rt_len",
            ret: Some(BuiltinRet::Int32),
        },
        "concat" => BuiltinMethodInfo {
            callee: "rt_String_concat",
            ret: Some(BuiltinRet::String),
        },
        "includes" => BuiltinMethodInfo {
            callee: "rt_String_includes",
            ret: Some(BuiltinRet::Bool),
        },
        "startsWith" => BuiltinMethodInfo {
            callee: "rt_String_startsWith",
            ret: Some(BuiltinRet::Bool),
        },
        "endsWith" => BuiltinMethodInfo {
            callee: "rt_String_endsWith",
            ret: Some(BuiltinRet::Bool),
        },
        "indexOf" => BuiltinMethodInfo {
            callee: "rt_String_indexOf",
            ret: Some(BuiltinRet::Int32),
        },
        "toUpperCase" => BuiltinMethodInfo {
            callee: "rt_String_toUpperCase",
            ret: Some(BuiltinRet::String),
        },
        "toLowerCase" => BuiltinMethodInfo {
            callee: "rt_String_toLowerCase",
            ret: Some(BuiltinRet::String),
        },
        "trim" => BuiltinMethodInfo {
            callee: "rt_String_trim",
            ret: Some(BuiltinRet::String),
        },
        "trimStart" => BuiltinMethodInfo {
            callee: "rt_String_trimStart",
            ret: Some(BuiltinRet::String),
        },
        "trimEnd" => BuiltinMethodInfo {
            callee: "rt_String_trimEnd",
            ret: Some(BuiltinRet::String),
        },
        "substring" => BuiltinMethodInfo {
            callee: "rt_String_substring",
            ret: Some(BuiltinRet::String),
        },
        "split" => BuiltinMethodInfo {
            callee: "rt_String_split",
            ret: Some(BuiltinRet::StringArray),
        },
        "repeat" => BuiltinMethodInfo {
            callee: "rt_String_repeat",
            ret: Some(BuiltinRet::String),
        },
        "replace" => BuiltinMethodInfo {
            callee: "rt_String_replace",
            ret: Some(BuiltinRet::String),
        },
        "padStart" => BuiltinMethodInfo {
            callee: "rt_String_padStart",
            ret: Some(BuiltinRet::String),
        },
        "padEnd" => BuiltinMethodInfo {
            callee: "rt_String_padEnd",
            ret: Some(BuiltinRet::String),
        },
        _ => return None,
    };
    Some(info)
}

fn array_method_info(name: &str) -> Option<BuiltinMethodInfo> {
    let info = match name {
        "length" => BuiltinMethodInfo {
            callee: "rt_len",
            ret: Some(BuiltinRet::Int32),
        },
        "push" => BuiltinMethodInfo {
            callee: "rt_array_push",
            ret: Some(BuiltinRet::Int32),
        },
        "pop" => BuiltinMethodInfo {
            callee: "rt_array_pop",
            ret: Some(BuiltinRet::Elem),
        },
        "shift" => BuiltinMethodInfo {
            callee: "rt_array_shift",
            ret: Some(BuiltinRet::Elem),
        },
        "unshift" => BuiltinMethodInfo {
            callee: "rt_array_unshift",
            ret: Some(BuiltinRet::Int32),
        },
        "indexOf" => BuiltinMethodInfo {
            callee: "rt_array_indexOf",
            ret: Some(BuiltinRet::Int32),
        },
        "concat" => BuiltinMethodInfo {
            callee: "rt_array_concat",
            ret: Some(BuiltinRet::SelfArray),
        },
        "join" => BuiltinMethodInfo {
            callee: "rt_array_join",
            ret: Some(BuiltinRet::String),
        },
        "slice" => BuiltinMethodInfo {
            callee: "rt_array_slice",
            ret: Some(BuiltinRet::SelfArray),
        },
        "reverse" => BuiltinMethodInfo {
            callee: "rt_array_reverse",
            ret: Some(BuiltinRet::SelfArray),
        },
        "fill" => BuiltinMethodInfo {
            callee: "rt_array_fill",
            ret: Some(BuiltinRet::SelfArray),
        },
        "sort" => BuiltinMethodInfo {
            callee: "rt_array_sort",
            ret: Some(BuiltinRet::Void),
        },
        "map" => BuiltinMethodInfo {
            callee: "f_map",
            ret: None,
        },
        "filter" => BuiltinMethodInfo {
            callee: "f_filter",
            ret: Some(BuiltinRet::SelfArray),
        },
        "forEach" => BuiltinMethodInfo {
            callee: "f_forEach",
            ret: Some(BuiltinRet::Void),
        },
        "reduce" => BuiltinMethodInfo {
            callee: "f_reduce",
            ret: None,
        },
        "every" => BuiltinMethodInfo {
            callee: "f_every",
            ret: Some(BuiltinRet::Bool),
        },
        "some" => BuiltinMethodInfo {
            callee: "f_some",
            ret: Some(BuiltinRet::Bool),
        },
        "find" => BuiltinMethodInfo {
            callee: "f_find",
            ret: Some(BuiltinRet::Elem),
        },
        "findIndex" => BuiltinMethodInfo {
            callee: "f_findIndex",
            ret: Some(BuiltinRet::Int32),
        },
        "includes" => BuiltinMethodInfo {
            callee: "f_includes",
            ret: Some(BuiltinRet::Bool),
        },
        _ => return None,
    };
    Some(info)
}

fn resolve_ret(receiver_ty: &TejxType, ret: BuiltinRet) -> TejxType {
    match ret {
        BuiltinRet::Int32 => TejxType::Int32,
        BuiltinRet::Bool => TejxType::Bool,
        BuiltinRet::String => TejxType::String,
        BuiltinRet::Void => TejxType::Void,
        BuiltinRet::Elem => receiver_ty.get_array_element_type(),
        BuiltinRet::SelfArray => receiver_ty.clone(),
        BuiltinRet::StringArray => TejxType::DynamicArray(Box::new(TejxType::String)),
    }
}

pub fn method_info(receiver_ty: &TejxType, name: &str) -> Option<(String, Option<TejxType>)> {
    if receiver_ty == &TejxType::String {
        let info = string_method_info(name)?;
        let ret = info.ret.map(|r| resolve_ret(receiver_ty, r));
        return Some((info.callee.to_string(), ret));
    }
    if receiver_ty.is_array() || receiver_ty.is_slice() {
        let info = array_method_info(name)?;
        let ret = info.ret.map(|r| resolve_ret(receiver_ty, r));
        return Some((info.callee.to_string(), ret));
    }
    None
}

pub fn member_names(receiver_ty: &TejxType) -> Option<&'static [&'static str]> {
    if receiver_ty == &TejxType::String {
        return Some(STRING_METHOD_NAMES);
    }
    if receiver_ty.is_array() || receiver_ty.is_slice() {
        return Some(ARRAY_METHOD_NAMES);
    }
    None
}

pub fn method_callee(receiver_ty: &TejxType, name: &str) -> Option<String> {
    method_info(receiver_ty, name).map(|(callee, _)| callee)
}

pub fn method_return_type(receiver_ty: &TejxType, name: &str) -> Option<TejxType> {
    method_info(receiver_ty, name).and_then(|(_, ret)| ret)
}
