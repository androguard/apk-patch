//! DEX text format utilities for apk-patch Phase 1.

mod access;
mod assemble;
mod emit;
mod insn_merge;
mod jobs;
mod naming;
mod parse;
mod resolver;

pub use assemble::{assemble_dex_from_project, AssembleOptions};
pub use emit::{emit_dex_to_dir, EmitOptions};
pub use naming::{dex_apk_name, dex_dir_name, is_odex, list_dex_dirs, DexDirEntry};
pub use parse::{parse_class_file, DexTxtClass, DexTxtMethod};

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
