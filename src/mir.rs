use crate::token::TokenType;
/// Mid-level Intermediate Representation (MIR), mirroring C++ MIR.h
/// SSA-like form with basic blocks and low-level instructions.
use crate::types::TejxType;

#[derive(Debug, Clone)]
pub enum MIRValue {
    Variable { name: String, ty: TejxType },
    Constant { value: String, ty: TejxType },
}

impl MIRValue {
    pub fn get_type(&self) -> &TejxType {
        match self {
            MIRValue::Variable { ty, .. } => ty,
            MIRValue::Constant { ty, .. } => ty,
        }
    }
}

#[derive(Debug, Clone)]
pub enum MIRInstruction {
    Move {
        dst: String, // destination variable name
        src: MIRValue,
        line: usize,
    },
    BinaryOp {
        dst: String,
        left: MIRValue,
        op: TokenType,
        right: MIRValue,
        line: usize,
    },
    Branch {
        condition: MIRValue,
        true_target: usize, // index into MIRFunction.blocks
        false_target: usize,
        line: usize,
    },
    Jump {
        target: usize, // index into MIRFunction.blocks
        line: usize,
    },
    Return {
        value: Option<MIRValue>,
        line: usize,
    },
    Call {
        dst: String,
        callee: String,
        args: Vec<MIRValue>,
        line: usize,
    },
    IndirectCall {
        dst: String,
        callee: MIRValue,
        args: Vec<MIRValue>,
        line: usize,
    },
    LoadMember {
        dst: String,
        obj: MIRValue,
        member: String,
        line: usize,
    },
    StoreMember {
        obj: MIRValue,
        member: String,
        src: MIRValue,
        line: usize,
    },
    LoadIndex {
        dst: String,
        obj: MIRValue,
        index: MIRValue,
        line: usize,
    },
    StoreIndex {
        obj: MIRValue,
        index: MIRValue,
        src: MIRValue,
        line: usize,
    },
    Throw {
        value: MIRValue,
        line: usize,
    },
    Cast {
        dst: String,
        src: MIRValue,
        ty: TejxType,
        line: usize,
    },
    TrySetup {
        try_target: usize,
        _catch_target: usize,
        line: usize,
    },
    PopHandler {
        line: usize,
    },
}

impl MIRInstruction {
    pub fn get_line(&self) -> usize {
        match self {
            MIRInstruction::Move { line, .. } => *line,
            MIRInstruction::BinaryOp { line, .. } => *line,
            MIRInstruction::Branch { line, .. } => *line,
            MIRInstruction::Jump { line, .. } => *line,
            MIRInstruction::Return { line, .. } => *line,
            MIRInstruction::Call { line, .. } => *line,
            MIRInstruction::IndirectCall { line, .. } => *line,
            MIRInstruction::LoadMember { line, .. } => *line,
            MIRInstruction::StoreMember { line, .. } => *line,
            MIRInstruction::LoadIndex { line, .. } => *line,
            MIRInstruction::StoreIndex { line, .. } => *line,
            MIRInstruction::Throw { line, .. } => *line,
            MIRInstruction::Cast { line, .. } => *line,
            MIRInstruction::TrySetup { line, .. } => *line,
            MIRInstruction::PopHandler { line, .. } => *line,
        }
    }

    pub fn set_line(&mut self, new_line: usize) {
        match self {
            MIRInstruction::Move { line, .. } => *line = new_line,
            MIRInstruction::BinaryOp { line, .. } => *line = new_line,
            MIRInstruction::Branch { line, .. } => *line = new_line,
            MIRInstruction::Jump { line, .. } => *line = new_line,
            MIRInstruction::Return { line, .. } => *line = new_line,
            MIRInstruction::Call { line, .. } => *line = new_line,
            MIRInstruction::IndirectCall { line, .. } => *line = new_line,
            MIRInstruction::LoadMember { line, .. } => *line = new_line,
            MIRInstruction::StoreMember { line, .. } => *line = new_line,
            MIRInstruction::LoadIndex { line, .. } => *line = new_line,
            MIRInstruction::StoreIndex { line, .. } => *line = new_line,
            MIRInstruction::Throw { line, .. } => *line = new_line,
            MIRInstruction::Cast { line, .. } => *line = new_line,
            MIRInstruction::TrySetup { line, .. } => *line = new_line,
            MIRInstruction::PopHandler { line, .. } => *line = new_line,
        }
    }
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
            matches!(
                last,
                MIRInstruction::Return { .. }
                    | MIRInstruction::Jump { .. }
                    | MIRInstruction::Branch { .. }
            )
        } else {
            false
        }
    }
}

#[derive(Debug, Clone)]
pub struct MIRFunction {
    pub name: String,
    pub params: Vec<String>, // parameter names
    pub variables: std::collections::HashMap<String, TejxType>, // variable types
    pub blocks: Vec<BasicBlock>,
    pub entry_block: usize, // index into blocks
    pub is_extern: bool,
}

impl MIRFunction {
    pub fn new(name: String) -> Self {
        Self {
            name,
            params: Vec::new(),
            variables: std::collections::HashMap::new(),
            blocks: Vec::new(),
            entry_block: 0,
            is_extern: false,
        }
    }
}
