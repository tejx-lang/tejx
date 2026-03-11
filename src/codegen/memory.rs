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
            // Use rt_len for .length access
            self.declare_runtime_fn("rt_len", "i64 @rt_len(i64)");
            self.emit_line(&format!("{} = call i64 @rt_len(i64 {})", res_tmp, obj_val));
            used_fast = true;
        } else {
            if let TejxType::Class(class_name, _) = obj.get_type() {
                let lookup_name = if class_name.contains('<') {
                    class_name.split('<').next().unwrap()
                } else {
                    class_name
                };
                let field_info = self.class_fields.get(lookup_name).and_then(|fields| {
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

                if let Some((offset, field_ty)) = field_info {
                    let llvm_ty = match field_ty {
                        TejxType::Bool => "i8",
                        TejxType::Int16 | TejxType::Float16 => "i16",
                        TejxType::Int32 | TejxType::Float32 => "i32",
                        _ => "i64",
                    };

                    let ptr_reg = format!("%ptr_{}", self.temp_counter);
                    self.temp_counter += 1;
                    let raw_obj = self.emit_strip_heap_offset(&obj_val);
                    self.emit_line(&format!("{} = inttoptr i64 {} to i8*", ptr_reg, raw_obj));

                    let field_ptr = format!("%field_ptr_{}", self.temp_counter);
                    self.temp_counter += 1;
                    self.emit_line(&format!(
                        "{} = getelementptr i8, i8* {}, i32 {}",
                        field_ptr, ptr_reg, offset
                    ));

                    let typed_field_ptr = format!("%typed_field_ptr_{}", self.temp_counter);
                    self.temp_counter += 1;
                    self.emit_line(&format!(
                        "{} = bitcast i8* {} to {}*",
                        typed_field_ptr, field_ptr, llvm_ty
                    ));

                    let loaded_val = format!("%loaded_val_{}", self.temp_counter);
                    self.temp_counter += 1;
                    self.emit_line(&format!(
                        "{} = load {}, {}* {}",
                        loaded_val, llvm_ty, llvm_ty, typed_field_ptr
                    ));

                    res_tmp = self.emit_abi_cast(&loaded_val, &field_ty, dst_ty);

                    used_fast = true;
                }
            }

            if !used_fast {
                let k_val = self.resolve_value(&MIRValue::Constant {
                    value: format!("\"{}\"", member),
                    ty: TejxType::String,
                });
                self.declare_runtime_fn("rt_map_get_fast", "i64 @rt_map_get_fast(i64, i64)");
                self.emit_line(&format!(
                    "{} = call i64 @rt_map_get_fast(i64 {}, i64 {})",
                    res_tmp, obj_val, k_val
                ));
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
                        if (field_ty.is_numeric()
                            || matches!(field_ty, TejxType::Bool | TejxType::Char))
                            && field_ty == dst_ty
                        {
                            needs_unboxing = false;
                        }
                    }
                }
            }
        }

        let final_res = if needs_unboxing && dst_ty.is_numeric() && !dst_ty.is_float() {
            self.declare_runtime_fn("rt_to_number", "double @rt_to_number(i64)");
            self.temp_counter += 1;
            let f_val = format!("%f_val_{}", self.temp_counter);
            self.emit_line(&format!(
                "{} = call double @rt_to_number(i64 {})",
                f_val, res_tmp
            ));
            self.temp_counter += 1;
            let i_val = format!("%i_val_{}", self.temp_counter);
            self.emit_line(&format!("{} = fptosi double {} to i64", i_val, f_val));
            i_val
        } else if needs_unboxing && dst_ty.is_float() {
            self.declare_runtime_fn("rt_to_number", "double @rt_to_number(i64)");
            self.temp_counter += 1;
            let f_val = format!("%f_val_{}", self.temp_counter);
            self.emit_line(&format!(
                "{} = call double @rt_to_number(i64 {})",
                f_val, res_tmp
            ));
            self.temp_counter += 1;
            let bc_val = format!("%bc_val_{}", self.temp_counter);
            self.emit_line(&format!("{} = bitcast double {} to i64", bc_val, f_val));
            bc_val
        } else if needs_unboxing && matches!(dst_ty, TejxType::Bool) {
            self.declare_runtime_fn("rt_to_boolean", "i64 @rt_to_boolean(i64)");
            self.temp_counter += 1;
            let b_val = format!("%b_val_{}", self.temp_counter);
            self.emit_line(&format!(
                "{} = call i64 @rt_to_boolean(i64 {})",
                b_val, res_tmp
            ));
            self.emit_abi_cast(&b_val, &TejxType::Int64, &TejxType::Bool)
        } else {
            res_tmp
        };

        self.emit_store_variable(dst, &final_res, dst_ty);
    }

    pub(crate) fn emit_store_member(
        &mut self,
        func: &MIRFunction,
        obj: &MIRValue,
        member: &String,
        src: &MIRValue,
    ) {
        let obj_val = self.resolve_value(obj);
        let k_val = self.resolve_value(&MIRValue::Constant {
            value: format!("\"{}\"", member),
            ty: TejxType::String,
        });

        let mut v_val = self.resolve_value(src);
        let v_ty = src.get_type();
        if matches!(v_ty, TejxType::String) && v_val.starts_with("ptrtoint") {
            v_val = self.emit_box_string(&v_val);
        }

        // Only box if target is not a known primitive field of a class
        let mut needs_boxing = true;
        let mut used_fast_store = false;
        if let TejxType::Class(class_name, _) = obj.get_type() {
            let lookup_name = if class_name.contains('<') {
                class_name.split('<').next().unwrap()
            } else {
                class_name
            };
            let field_info = self.class_fields.get(lookup_name).and_then(|fields| {
                fields.iter().position(|(f, _)| f == member).map(|pos| {
                    let mut offset = 0;
                    for (_, ty) in &fields[..pos] {
                        offset = Self::get_aligned_offset(offset, ty);
                        offset += ty.size();
                    }
                    let field_ty = fields[pos].1.clone();
                    offset = Self::get_aligned_offset(offset, &field_ty);

                    (field_ty, offset)
                })
            });

            if let Some((field_ty, offset)) = field_info {
                needs_boxing = false;

                let llvm_ty = match field_ty {
                    TejxType::Bool => "i8",
                    TejxType::Int16 | TejxType::Float16 => "i16",
                    TejxType::Int32 | TejxType::Float32 => "i32",
                    _ => "i64",
                };

                let ptr_reg = format!("%ptr_store_{}", self.temp_counter);
                self.temp_counter += 1;
                let raw_obj = self.emit_strip_heap_offset(&obj_val);
                self.emit_line(&format!("{} = inttoptr i64 {} to i8*", ptr_reg, raw_obj));

                let field_ptr = format!("%field_ptr_store_{}", self.temp_counter);
                self.temp_counter += 1;
                self.emit_line(&format!(
                    "{} = getelementptr i8, i8* {}, i32 {}",
                    field_ptr, ptr_reg, offset
                ));

                let typed_field_ptr = format!("%typed_field_ptr_store_{}", self.temp_counter);
                self.temp_counter += 1;
                self.emit_line(&format!(
                    "{} = bitcast i8* {} to {}*",
                    typed_field_ptr, field_ptr, llvm_ty
                ));

                let casted_val = self.emit_abi_cast(&v_val, &v_ty, &field_ty);
                let truncated_val = casted_val;
                self.emit_line(&format!(
                    "store {} {}, {}* {}",
                    llvm_ty, truncated_val, llvm_ty, typed_field_ptr
                ));
                used_fast_store = true;
            }
        }

        // If not a static class field, check if it's an array index
        if !used_fast_store && obj.get_type().is_array() {
            if let Ok(idx) = member.parse::<i64>() {
                self.declare_runtime_fn(
                    "rt_array_set_fast",
                    "void @rt_array_set_fast(i64, i64, i64) nounwind",
                );
                self.emit_line(&format!(
                    "call void @rt_array_set_fast(i64 {}, i64 {}, i64 {})",
                    obj_val, idx, v_val
                ));
                used_fast_store = true;
            }
        }

        if !used_fast_store
            && needs_boxing
            && (v_ty.is_numeric() || matches!(v_ty, TejxType::Bool | TejxType::Char))
        {
            self.temp_counter += 1;
            let boxed_reg = format!("%boxed_set_{}", self.temp_counter);

            // Primitives are now bitcasted directly into i64 slots (generic slots)
            if v_ty.is_float() {
                self.emit_line(&format!("{} = bitcast double {} to i64", boxed_reg, v_val));
            } else {
                let casted = self.emit_abi_cast(&v_val, v_ty, &TejxType::Int64);
                self.emit_line(&format!("{} = or i64 0, {}", boxed_reg, casted));
            }
            v_val = boxed_reg;
        }

        if !used_fast_store {
            self.declare_runtime_fn("rt_Map_set", "void @rt_Map_set(i64, i64, i64) nounwind");
            self.emit_line(&format!(
                "call void @rt_Map_set(i64 {}, i64 {}, i64 {})",
                obj_val, k_val, v_val
            ));
        }
    }

    pub(crate) fn emit_load_index(
        &mut self,
        func: &MIRFunction,
        dst: &String,
        obj: &MIRValue,
        index: &MIRValue,
    ) {
        let obj_val = self.resolve_value(obj);
        let idx_val = self.resolve_value(index);
        let idx_val = self.emit_abi_cast(&idx_val, index.get_type(), &TejxType::Int64);
        self.temp_counter += 1;
        let res_tmp = format!("%val{}", self.temp_counter);

        self.declare_runtime_fn(
            "rt_array_get_fast",
            "i64 @rt_array_get_fast(i64, i64) nounwind",
        );
        self.emit_line(&format!(
            "{} = call i64 @rt_array_get_fast(i64 {}, i64 {})",
            res_tmp, obj_val, idx_val
        ));
        let dst_ty = func.variables.get(dst).unwrap_or(&TejxType::Void);
        let casted = self.emit_abi_cast(&res_tmp, &TejxType::Int64, dst_ty);
        self.emit_store_variable(dst, &casted, dst_ty);
    }

    pub(crate) fn emit_store_index(
        &mut self,
        func: &MIRFunction,
        obj: &MIRValue,
        index: &MIRValue,
        src: &MIRValue,
    ) {
        let obj_val = self.resolve_value(obj);
        let idx_val = self.resolve_value(index);
        let mut v_val = self.resolve_value(src);

        let idx_val = self.emit_abi_cast(&idx_val, index.get_type(), &TejxType::Int64);
        let src_ty = src.get_type();
        v_val = self.emit_abi_cast(&v_val, src_ty, &TejxType::Int64);

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
