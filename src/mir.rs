/// Mid-level Intermediate Representation (MIR), mirroring C++ MIR.h
/// SSA-like form with basic blocks and low-level instructions.

use crate::types::TejxType;
use crate::token::TokenType;

#[derive(Debug, Clone)]
pub enum MIRValue {
    Variable {
        name: String,
        ty: TejxType,
    },
    Constant {
        value: String,
        ty: TejxType,
    },
}

impl MIRValue {
    pub fn get_type(&self) -> &TejxType {
        match self {
            MIRValue::Variable { ty, .. } => ty,
            MIRValue::Constant { ty, .. } => ty,
        }
    }

    pub fn get_name(&self) -> String {
        match self {
            MIRValue::Variable { name, .. } => name.clone(),
            MIRValue::Constant { value, .. } => value.clone(),
        }
    }
}


#[derive(Debug, Clone)]
pub enum MIRInstruction {
    Move {
        dst: String,    // destination variable name
        src: MIRValue,
    },
    BinaryOp {
        dst: String,
        left: MIRValue,
        op: TokenType,
        right: MIRValue,
    },
    Branch {
        condition: MIRValue,
        true_target: usize,   // index into MIRFunction.blocks
        false_target: usize,
    },
    Jump {
        target: usize,        // index into MIRFunction.blocks
    },
    Return {
        value: Option<MIRValue>,
    },
    Call {
        dst: String,
        callee: String,
        args: Vec<MIRValue>,
    },
    IndirectCall {
        dst: String,
        callee: MIRValue,
        args: Vec<MIRValue>,
    },
    ObjectLiteral {
        dst: String,
        entries: Vec<(String, MIRValue)>,
        ty: Option<TejxType>,
    },
    ArrayLiteral {
        dst: String,
        elements: Vec<MIRValue>,
        ty: Option<TejxType>,
    },
    LoadMember {
        dst: String,
        obj: MIRValue,
        member: String,
    },
    StoreMember {
        obj: MIRValue,
        member: String,
        src: MIRValue,
    },
    LoadIndex {
        dst: String,
        obj: MIRValue,
        index: MIRValue,
    },
    StoreIndex {
        obj: MIRValue,
        index: MIRValue,
        src: MIRValue,
    },
    Throw {
        value: MIRValue,
    },
    Cast {
        dst: String,
        src: MIRValue,
        ty: TejxType,
    },
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub name: String,
    pub instructions: Vec<MIRInstruction>,
    pub exception_handler: Option<usize>, // target block index if an exception occurs in this block
}

impl BasicBlock {
    pub fn new(name: String) -> Self {
        Self {
            name,
            instructions: Vec::new(),
            exception_handler: None,
        }
    }

    pub fn add_instruction(&mut self, inst: MIRInstruction) {
        self.instructions.push(inst);
    }

    pub fn is_terminated(&self) -> bool {
        if let Some(last) = self.instructions.last() {
            matches!(last,
                MIRInstruction::Return { .. } |
                MIRInstruction::Jump { .. } |
                MIRInstruction::Branch { .. }
            )
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
pub struct MIRFunction {
    pub name: String,
    pub params: Vec<String>,  // parameter names
    pub variables: std::collections::HashMap<String, TejxType>, // variable types
    pub blocks: Vec<BasicBlock>,
    pub entry_block: usize,  // index into blocks
}

impl MIRFunction {
    pub fn new(name: String) -> Self {
        Self {
            name,
            params: Vec::new(),
            variables: std::collections::HashMap::new(),
            blocks: Vec::new(),
            entry_block: 0,
        }
    }
}
