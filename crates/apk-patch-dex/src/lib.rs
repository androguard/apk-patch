//! DEX text format utilities for apk-patch Phase 1.

mod access;
mod assemble;
mod emit;
mod insn_merge;
mod jobs;
mod naming;
mod parse;
mod payloads;
mod resolver;
mod verify;

pub use assemble::{
    assemble_dex_from_project, assemble_dex_from_txt, assemble_dex_from_vfs, AssembleOptions,
};
pub use emit::{emit_dex_to_dir, emit_dex_to_vfs, EmitOptions};
pub use naming::{
    dex_apk_name, dex_dir_name, is_odex, list_dex_dirs, list_dex_dirs_vfs, DexDirEntry,
};
pub use parse::{
    parse_class_file, DexTxtAnnotation, DexTxtCatch, DexTxtClass, DexTxtDebug, DexTxtField,
    DexTxtInsn, DexTxtMethod,
};
pub use verify::{
    analyze_method, locals_to_registers, registers_to_locals, verify_class, verify_method,
    verify_method_structural, Category, RegType,
};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum DexError {
    #[error(transparent)]
    Parser(#[from] dex_parser::DexError),
    #[error(transparent)]
    Bytecode(#[from] dex_bytecode::DexError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("dex-txt error: {0}")]
    Txt(String),
}

pub type Result<T> = std::result::Result<T, DexError>;

/// Returns true if bytes look like an odex/DEY file.
pub fn is_odex_bytes(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == b"dey\n"
}
