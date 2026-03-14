use super::*;
use crate::mir::*;
use crate::types::TejxType;

impl CodeGen {
    pub(crate) fn emit_load_member(
        &mut self,
        func: &MIRFunction,
        dst: &String,
        obj: &MIRValue,
        member: &String,
    ) {
        let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
        let obj_val = self.resolve_value(obj);
        self.temp_counter += 1;
        let mut res_tmp = format!("%val{}", self.temp_counter);

        let mut used_fast = false;
        if member == "length" {
            self.declare_runtime_fn("rt_len", "i64 @rt_len(i64)");
            self.emit_line(&format!("{} = call i64 @rt_len(i64 {})", res_tmp, obj_val));
            used_fast = true;
        } else {
            let mut resolved_info = None;
            if let TejxType::Class(class_name, _) = obj.get_type() {
                let lookup_name = if class_name.contains('<') {
                    class_name.split('<').next().unwrap()
                } else {
                    &class_name
                };
                resolved_info = self.class_fields.get(lookup_name).and_then(|fields| {
                    fields.iter().position(|(f, _)| f == member).map(|pos| {
                        let mut offset = 0;
                        for (_, ty) in &fields[..pos] {
                            offset = Self::get_aligned_offset(offset, ty);
                            offset += ty.size();
                        }
                        let field_ty = fields[pos].1.clone();
                        offset = Self::get_aligned_offset(offset, &field_ty);
                        (offset, field_ty)
                    })
                });

                if let Some((offset, field_ty)) = resolved_info {
                    let llvm_ty = Self::get_llvm_storage_type(&field_ty);

                    let ptr_reg = format!("%ptr_{}", self.temp_counter);
                    self.temp_counter += 1;
                    let raw_obj = self.emit_strip_heap_offset(&obj_val);
                    self.emit_line(&format!("{} = inttoptr i64 {} to i8*", ptr_reg, raw_obj));

                    let field_ptr = format!("%field_ptr_{}", self.temp_counter);
                    self.temp_counter += 1;
                    self.emit_line(&format!("{} = getelementptr i8, i8* {}, i32 {}", field_ptr, ptr_reg, offset));

                    let typed_field_ptr = format!("%typed_field_ptr_{}", self.temp_counter);
                    self.temp_counter += 1;
                    self.emit_line(&format!("{} = bitcast i8* {} to {}*", typed_field_ptr, field_ptr, llvm_ty));

                    let loaded_val = format!("%loaded_val_{}", self.temp_counter);
                    self.temp_counter += 1;
                    self.emit_line(&format!("{} = load {}, {}* {}", loaded_val, llvm_ty, llvm_ty, typed_field_ptr));

                    let mut value_val = loaded_val;
                    if llvm_ty == "i8" && matches!(field_ty, TejxType::Bool) {
                        value_val = self.emit_storage_to_value(&value_val, &field_ty);
                    }
                    res_tmp = self.emit_abi_cast(&value_val, &field_ty, dst_ty);
                    used_fast = true;
                }
            }

            if !used_fast {
                let mut temp_root_count = 0;
                if Self::is_gc_managed(obj.get_type())
                    && !(matches!(obj.get_type(), TejxType::String)
                        && obj_val.starts_with("ptrtoint"))
                {
                    self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
                    self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
                    self.temp_counter += 1;
                    let tmp_root = format!("%member_obj_root_{}", self.temp_counter);
                    self.alloca_buffer
                        .push_str(&format!("  {} = alloca i64\n", tmp_root));
                    self.emit_line(&format!("store i64 {}, i64* {}", obj_val, tmp_root));
                    self.emit_line(&format!("call void @rt_push_root(i64* {})", tmp_root));
                    temp_root_count += 1;
                }

                let k_val = self.resolve_value(&MIRValue::Constant {
                    value: format!("\"{}\"", member),
                    ty: TejxType::String,
                });
                if Self::is_gc_managed(&TejxType::String) && !k_val.starts_with("ptrtoint") {
                    self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
                    self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
                    self.temp_counter += 1;
                    let tmp_root = format!("%member_key_root_{}", self.temp_counter);
                    self.alloca_buffer
                        .push_str(&format!("  {} = alloca i64\n", tmp_root));
                    self.emit_line(&format!("store i64 {}, i64* {}", k_val, tmp_root));
                    self.emit_line(&format!("call void @rt_push_root(i64* {})", tmp_root));
                    temp_root_count += 1;
                }
                self.declare_runtime_fn("rt_get_property", "i64 @rt_get_property(i64, i64)");
                self.emit_line(&format!("{} = call i64 @rt_get_property(i64 {}, i64 {})", res_tmp, obj_val, k_val));

                if temp_root_count > 0 {
                    self.emit_line(&format!(
                        "call void @rt_pop_roots(i64 {})",
                        temp_root_count
                    ));
                }
            }
        }

        let mut needs_unboxing = !used_fast;
        if !used_fast {
            if let TejxType::Class(class_name, _) = obj.get_type() {
                let lookup_name = if class_name.contains('<') {
                    class_name.split('<').next().unwrap()
                } else {
                    class_name
                };
                if let Some(fields) = self.class_fields.get(lookup_name) {
                    if let Some((_, field_ty)) = fields.iter().find(|(f, _)| f == member) {
                        if (field_ty.is_numeric() || matches!(field_ty, TejxType::Bool | TejxType::Char)) && field_ty == dst_ty {
                            needs_unboxing = false;
                        }
                    }
                }
            }
        }

        let final_res = if needs_unboxing && dst_ty.is_numeric() && !matches!(dst_ty, TejxType::Int64 | TejxType::Any) {
            self.declare_runtime_fn("rt_to_number", "double @rt_to_number(i64)");
            self.temp_counter += 1;
            let f_val = format!("%f_val_{}", self.temp_counter);
            self.emit_line(&format!("{} = call double @rt_to_number(i64 {})", f_val, res_tmp));
            self.temp_counter += 1;
            let i_val = format!("%i_val_{}", self.temp_counter);
            self.emit_line(&format!("{} = fptosi double {} to i64", i_val, f_val));
            self.emit_abi_cast(&i_val, &TejxType::Int64, dst_ty)
        } else if needs_unboxing && dst_ty.is_float() {
            self.declare_runtime_fn("rt_to_number", "double @rt_to_number(i64)");
            self.temp_counter += 1;
            let f_val = format!("%f_val_{}", self.temp_counter);
            self.emit_line(&format!("{} = call double @rt_to_number(i64 {})", f_val, res_tmp));
            self.temp_counter += 1;
            let bc_val = format!("%bc_val_{}", self.temp_counter);
            self.emit_line(&format!("{} = bitcast double {} to i64", bc_val, f_val));
            bc_val
        } else if needs_unboxing && matches!(dst_ty, TejxType::Bool) {
            self.declare_runtime_fn("rt_to_boolean", "i64 @rt_to_boolean(i64)");
            self.temp_counter += 1;
            let b_val = format!("%b_val_{}", self.temp_counter);
            self.emit_line(&format!("{} = call i64 @rt_to_boolean(i64 {})", b_val, res_tmp));
            self.emit_abi_cast(&b_val, &TejxType::Int64, &TejxType::Bool)
        } else {
            res_tmp
        };

        self.emit_store_variable(dst, &final_res, dst_ty);
    }

    pub(crate) fn emit_store_member(
        &mut self,
        _func: &MIRFunction,
        obj: &MIRValue,
        member: &String,
        src: &MIRValue,
    ) {
        let obj_val = self.resolve_value(obj);
        let mut v_val = self.resolve_value(src);
        let v_ty = src.get_type();
        if matches!(v_ty, TejxType::String) && v_val.starts_with("ptrtoint") {
            v_val = self.emit_box_string(&v_val);
        }

        let mut used_fast_store = false;
        let mut resolved_info = None;

        if let TejxType::Class(class_name, _) = obj.get_type() {
            let lookup_name = if class_name.contains('<') {
                class_name.split('<').next().unwrap()
            } else {
                &class_name
            };
            resolved_info = self.class_fields.get(lookup_name).and_then(|fields| {
                fields.iter().position(|(f, _)| f == member).map(|pos| {
                    let mut offset = 0;
                    for (_, ty) in &fields[..pos] {
                        offset = Self::get_aligned_offset(offset, ty);
                        offset += ty.size();
                    }
                    let field_ty = fields[pos].1.clone();
                    offset = Self::get_aligned_offset(offset, &field_ty);
                    (offset, field_ty)
                })
            });

            if let Some((offset, field_ty)) = resolved_info {
                let llvm_ty = Self::get_llvm_storage_type(&field_ty);

                let ptr_reg = format!("%ptr_store_{}", self.temp_counter);
                self.temp_counter += 1;
                let raw_obj = self.emit_strip_heap_offset(&obj_val);
                self.emit_line(&format!("{} = inttoptr i64 {} to i8*", ptr_reg, raw_obj));

                let field_ptr = format!("%field_ptr_store_{}", self.temp_counter);
                self.temp_counter += 1;
                self.emit_line(&format!("{} = getelementptr i8, i8* {}, i32 {}", field_ptr, ptr_reg, offset));

                let typed_field_ptr = format!("%typed_field_ptr_store_{}", self.temp_counter);
                self.temp_counter += 1;
                self.emit_line(&format!("{} = bitcast i8* {} to {}*", typed_field_ptr, field_ptr, llvm_ty));

                let final_src = self.emit_abi_cast(&v_val, &v_ty, &field_ty);
                let store_val = if llvm_ty == "i8" && matches!(field_ty, TejxType::Bool) {
                    self.emit_value_to_storage(&final_src, &field_ty)
                } else {
                    final_src
                };
                self.emit_line(&format!("store {} {}, {}* {}", llvm_ty, store_val, llvm_ty, typed_field_ptr));
                used_fast_store = true;
            }
        }

        if !used_fast_store {
            let k_val = self.resolve_value(&MIRValue::Constant {
                value: format!("\"{}\"", member),
                ty: TejxType::String,
            });
            let boxed_v = self.emit_auto_box(&v_val, &v_ty);
            let mut temp_root_count = 0;
            if Self::is_gc_managed(obj.get_type()) {
                self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
                self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
                self.temp_counter += 1;
                let tmp_root = format!("%member_obj_root_{}", self.temp_counter);
                self.alloca_buffer
                    .push_str(&format!("  {} = alloca i64\n", tmp_root));
                self.emit_line(&format!("store i64 {}, i64* {}", obj_val, tmp_root));
                self.emit_line(&format!("call void @rt_push_root(i64* {})", tmp_root));
                temp_root_count += 1;
            }

            if Self::is_gc_managed(&TejxType::String) {
                self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
                self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
                self.temp_counter += 1;
                let tmp_root = format!("%member_key_root_{}", self.temp_counter);
                self.alloca_buffer
                    .push_str(&format!("  {} = alloca i64\n", tmp_root));
                self.emit_line(&format!("store i64 {}, i64* {}", k_val, tmp_root));
                self.emit_line(&format!("call void @rt_push_root(i64* {})", tmp_root));
                temp_root_count += 1;
            }

            // boxed_v is always a GC-managed object (auto-boxed to Any).
            self.declare_runtime_fn("rt_push_root", "void @rt_push_root(i64*) nounwind");
            self.declare_runtime_fn("rt_pop_roots", "void @rt_pop_roots(i64) nounwind");
            self.temp_counter += 1;
            let tmp_root = format!("%member_val_root_{}", self.temp_counter);
            self.alloca_buffer
                .push_str(&format!("  {} = alloca i64\n", tmp_root));
            self.emit_line(&format!("store i64 {}, i64* {}", boxed_v, tmp_root));
            self.emit_line(&format!("call void @rt_push_root(i64* {})", tmp_root));
            temp_root_count += 1;

            self.declare_runtime_fn("rt_set_property", "void @rt_set_property(i64, i64, i64)");
            self.emit_line(&format!("call void @rt_set_property(i64 {}, i64 {}, i64 {})", obj_val, k_val, boxed_v));

            if temp_root_count > 0 {
                self.emit_line(&format!(
                    "call void @rt_pop_roots(i64 {})",
                    temp_root_count
                ));
            }
        }
    }

    pub(crate) fn emit_load_index(
        &mut self,
        func: &MIRFunction,
        dst: &String,
        obj: &MIRValue,
        index: &MIRValue,
        element_ty: &TejxType,
    ) {
        let obj_val = self.resolve_value(obj);
        let idx_val = self.resolve_value(index);
        let idx_val = self.emit_abi_cast(&idx_val, index.get_type(), &TejxType::Int64);
        self.temp_counter += 1;
        let res_tmp = format!("%val{}", self.temp_counter);

        if matches!(obj.get_type(), TejxType::String) {
            self.declare_runtime_fn(
                "rt_String_substring",
                "i64 @rt_String_substring(i64, i64, i64)",
            );
            self.temp_counter += 1;
            let idx_next = format!("%idx_next{}", self.temp_counter);
            self.emit_line(&format!("{} = add i64 {}, 1", idx_next, idx_val));
            self.emit_line(&format!(
                "{} = call i64 @rt_String_substring(i64 {}, i64 {}, i64 {})",
                res_tmp, obj_val, idx_val, idx_next
            ));
            let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
            let casted = self.emit_abi_cast(&res_tmp, &TejxType::String, dst_ty);
            self.emit_store_variable(dst, &casted, dst_ty);
            return;
        }

        self.declare_runtime_fn(
            "rt_array_get_fast",
            "i64 @rt_array_get_fast(i64, i64) nounwind",
        );
        self.emit_line(&format!(
            "{} = call i64 @rt_array_get_fast(i64 {}, i64 {})",
            res_tmp, obj_val, idx_val
        ));
        let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
        if element_ty.is_float() {
            if matches!(element_ty, TejxType::Float32) {
                self.temp_counter += 1;
                let trunc_i32 = format!("%f_trunc_{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = trunc i64 {} to i32",
                    trunc_i32, res_tmp
                ));
                self.temp_counter += 1;
                let f_val = format!("%f_val_{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = bitcast i32 {} to float",
                    f_val, trunc_i32
                ));
                let casted = self.emit_abi_cast(&f_val, &TejxType::Float32, dst_ty);
                self.emit_store_variable(dst, &casted, dst_ty);
            } else {
                self.temp_counter += 1;
                let f_val = format!("%f_val_{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = bitcast i64 {} to double",
                    f_val, res_tmp
                ));
                let casted = self.emit_abi_cast(&f_val, &TejxType::Float64, dst_ty);
                self.emit_store_variable(dst, &casted, dst_ty);
            }
        } else {
            let casted = self.emit_abi_cast(&res_tmp, &TejxType::Int64, dst_ty);
            self.emit_store_variable(dst, &casted, dst_ty);
        }
    }

    pub(crate) fn emit_store_index(
        &mut self,
        _func: &MIRFunction,
        obj: &MIRValue,
        index: &MIRValue,
        src: &MIRValue,
        element_ty: &TejxType,
    ) {
        let obj_val = self.resolve_value(obj);
        let idx_val = self.resolve_value(index);
        let mut v_val = self.resolve_value(src);

        let idx_val = self.emit_abi_cast(&idx_val, index.get_type(), &TejxType::Int64);
        let src_ty = src.get_type();
        if element_ty.is_float() {
            if matches!(element_ty, TejxType::Float32) {
                let f_val = self.emit_abi_cast(&v_val, src_ty, &TejxType::Float32);
                self.temp_counter += 1;
                let bits_i32 = format!("%f_bits_{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = bitcast float {} to i32",
                    bits_i32, f_val
                ));
                self.temp_counter += 1;
                let bits_i64 = format!("%f_bits64_{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = zext i32 {} to i64",
                    bits_i64, bits_i32
                ));
                v_val = bits_i64;
            } else {
                let f_val = self.emit_abi_cast(&v_val, src_ty, &TejxType::Float64);
                self.temp_counter += 1;
                let bits_i64 = format!("%f_bits64_{}", self.temp_counter);
                self.emit_line(&format!(
                    "{} = bitcast double {} to i64",
                    bits_i64, f_val
                ));
                v_val = bits_i64;
            }
        } else {
            v_val = self.emit_abi_cast(&v_val, src_ty, &TejxType::Int64);
        }

        self.declare_runtime_fn(
            "rt_array_set_fast",
            "void @rt_array_set_fast(i64, i64, i64) nounwind",
        );
        self.emit_line(&format!(
            "call void @rt_array_set_fast(i64 {}, i64 {}, i64 {})",
            obj_val, idx_val, v_val
        ));
    }
}
