//! Merge dex-txt instruction patches into method bytecode with branch fixup.

use std::collections::HashMap;

use dex_bytecode::{branch_targets, decode_one, patch_branch_target};

use crate::{DexError, Result};

/// Merge sparse dex-txt patches into original method insns, allowing variable-size
/// replacements, then fix branch targets for the new layout.
pub fn merge_method_insns(original: &[u8], patches: &[(u32, Vec<u8>)]) -> Result<Vec<u8>> {
    if patches.is_empty() {
        return Ok(original.to_vec());
    }
    let patch_map: HashMap<u32, Vec<u8>> = patches
        .iter()
        .map(|(offset, bytes)| (*offset, bytes.clone()))
        .collect();

    let mut old_to_new: HashMap<u32, u32> = HashMap::new();
    let mut out = Vec::new();
    let mut old_off = 0usize;

    while old_off < original.len() {
        let old_start = old_off as u32;
        old_to_new.insert(old_start, out.len() as u32);

        if let Some(new_bytes) = patch_map.get(&old_start) {
            let orig_len = decode_one(original, old_off)
                .map(|ins| ins.length as usize)
                .unwrap_or(new_bytes.len());
            out.extend_from_slice(new_bytes);
            old_off += orig_len;
        } else {
            let ins = decode_one(original, old_off).map_err(DexError::Bytecode)?;
            let len = ins.length as usize;
            out.extend_from_slice(&original[old_off..old_off + len]);
            old_off += len;
        }
    }

    let needs_fixup = old_to_new.iter().any(|(old, new)| old != new);
    if needs_fixup {
        fix_branch_targets(original, &mut out, &patch_map, &old_to_new)?;
    }
    Ok(out)
}

fn fix_branch_targets(
    original: &[u8],
    new: &mut Vec<u8>,
    patch_map: &HashMap<u32, Vec<u8>>,
    old_to_new: &HashMap<u32, u32>,
) -> Result<()> {
    let mut old_off = 0usize;
    let mut new_off = 0usize;

    while old_off < original.len() {
        let old_start = old_off as u32;
        let new_start = new_off as u32;

        if let Some(patch) = patch_map.get(&old_start) {
            fix_patch_branches(old_start, new_start, patch, old_to_new, new)?;
            let orig_len = decode_one(original, old_off)
                .map(|ins| ins.length as usize)
                .unwrap_or(patch.len());
            old_off += orig_len;
            new_off += patch.len();
        } else {
            for abs_target in branch_targets(original, old_off) {
                let new_target = map_target(abs_target, old_to_new)?;
                let _ = patch_branch_target(new, new_off, new_target);
            }
            let ins = decode_one(original, old_off).map_err(DexError::Bytecode)?;
            let len = ins.length as usize;
            old_off += len;
            new_off += len;
        }
    }

    Ok(())
}

fn fix_patch_branches(
    old_start: u32,
    new_start: u32,
    patch: &[u8],
    old_to_new: &HashMap<u32, u32>,
    new: &mut Vec<u8>,
) -> Result<()> {
    let mut patch_off = 0usize;
    while patch_off < patch.len() {
        for abs_target in branch_targets(patch, patch_off) {
            let old_abs = old_start.saturating_add(abs_target);
            let new_target = map_target(old_abs, old_to_new)?;
            let new_branch_off = new_start as usize + patch_off;
            let _ = patch_branch_target(new, new_branch_off, new_target);
        }
        let ins = decode_one(patch, patch_off).map_err(DexError::Bytecode)?;
        patch_off += ins.length as usize;
    }
    Ok(())
}

fn map_target(old_abs: u32, old_to_new: &HashMap<u32, u32>) -> Result<u32> {
    if let Some(&mapped) = old_to_new.get(&old_abs) {
        return Ok(mapped);
    }
    old_to_new
        .iter()
        .filter(|(start, _)| **start <= old_abs)
        .max_by_key(|(start, _)| *start)
        .map(|(_, new_start)| *new_start)
        .ok_or_else(|| DexError::Txt(format!("no mapped branch target for 0x{old_abs:08x}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_grows_return_void_to_const16() {
        let original = [0x0e, 0x00]; // return-void
        let patches = vec![(0, vec![0x13, 0x00, 0x00, 0x00])]; // const/16 v0, 0
        let merged = merge_method_insns(&original, &patches).unwrap();
        assert_eq!(merged, vec![0x13, 0x00, 0x00, 0x00]);
    }
}
