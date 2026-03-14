use super::Lowering;
use crate::builtins;
use crate::types::TejxType;

impl Lowering {
    pub(crate) fn builtin_method_return_type(
        &self,
        receiver_ty: &TejxType,
        member: &str,
    ) -> Option<TejxType> {
        builtins::method_return_type(receiver_ty, member)
    }

    pub(crate) fn resolve_builtin_method_callee(
        &self,
        receiver_ty: &TejxType,
        member: &str,
    ) -> Option<String> {
        builtins::method_callee(receiver_ty, member)
    }
}
