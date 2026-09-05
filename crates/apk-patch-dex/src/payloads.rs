//! Encode/decode mnemonic switch and array payloads for dex-txt.

use std::collections::HashMap;

use crate::DexError;

/// Size in code units for a payload insn (does not resolve labels).
pub fn payload_size_units(mnemonic: &str, operands: &str) -> Result<u32, DexError> {
    let bytes = match mnemonic {
        ".array-data" => encode_array_data(operands)?,
        ".packed-switch" => {
            // ident(2) + size(2) + first_key(4) + targets(size*4)
            let lines: Vec<&str> = operands
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            if lines.is_empty() {
                return Err(DexError::Txt("empty .packed-switch".into()));
            }
            let n = lines.len() - 1; // exclude first_key line
            let mut out = vec![0u8; 8 + n * 4];
            out[0..2].copy_from_slice(&0x0100u16.to_le_bytes());
            out
        }
        ".sparse-switch" => {
            let n = operands
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .count();
            vec![0u8; 4 + n * 8]
        }
        _ => {
            return Err(DexError::Txt(format!(
                "not a payload mnemonic: {mnemonic}"
            )))
        }
    };
    Ok((bytes.len() as u32) / 2)
}

/// Encode payload; `switch_base_units` is the packed/sparse-switch instruction offset
/// (targets are relative to the switch, not the payload). Ignored for array-data.
pub fn encode_payload_bytes(
    mnemonic: &str,
    operands: &str,
    switch_base_units: u32,
    labels: &HashMap<String, u32>,
) -> Result<Vec<u8>, DexError> {
    match mnemonic {
        ".array-data" => encode_array_data(operands),
        ".packed-switch" => encode_packed_switch(operands, switch_base_units, labels),
        ".sparse-switch" => encode_sparse_switch(operands, switch_base_units, labels),
        _ => Err(DexError::Txt(format!("not a payload mnemonic: {mnemonic}"))),
    }
}

fn encode_array_data(operands: &str) -> Result<Vec<u8>, DexError> {
    let lines: Vec<&str> = operands
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Err(DexError::Txt("empty .array-data".into()));
    }
    let width = parse_int(lines[0])? as u16;
    if !matches!(width, 1 | 2 | 4 | 8) {
        return Err(DexError::Txt(format!(
            ".array-data width must be 1/2/4/8, got {width}"
        )));
    }
    let mut values = Vec::new();
    for line in &lines[1..] {
        for tok in line.split_whitespace() {
            values.push(parse_int(tok)?);
        }
    }
    let size = values.len() as u32;
    let mut out = Vec::new();
    out.extend_from_slice(&0x0300u16.to_le_bytes()); // ident
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    for v in values {
        match width {
            1 => out.push(v as u8),
            2 => out.extend_from_slice(&(v as i16 as u16).to_le_bytes()),
            4 => out.extend_from_slice(&(v as i32).to_le_bytes()),
            8 => out.extend_from_slice(&v.to_le_bytes()),
            _ => unreachable!(),
        }
    }
    if out.len() % 2 != 0 {
        out.push(0);
    }
    Ok(out)
}

fn encode_packed_switch(
    operands: &str,
    switch_base: u32,
    labels: &HashMap<String, u32>,
) -> Result<Vec<u8>, DexError> {
    let lines: Vec<&str> = operands
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Err(DexError::Txt("empty .packed-switch".into()));
    }
    let first_key = parse_int(lines[0])? as i32;
    let mut targets = Vec::new();
    for line in &lines[1..] {
        for tok in line.split_whitespace() {
            targets.push(resolve_target(tok, switch_base, labels)?);
        }
    }
    let size = targets.len() as u16;
    let mut out = Vec::new();
    out.extend_from_slice(&0x0100u16.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&first_key.to_le_bytes());
    for t in targets {
        out.extend_from_slice(&t.to_le_bytes());
    }
    Ok(out)
}

fn encode_sparse_switch(
    operands: &str,
    switch_base: u32,
    labels: &HashMap<String, u32>,
) -> Result<Vec<u8>, DexError> {
    // lines: `key -> :Ltarget` or `key :Ltarget`
    let mut keys = Vec::new();
    let mut targets = Vec::new();
    for line in operands.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (k, t) = if let Some((a, b)) = line.split_once("->") {
            (a.trim(), b.trim())
        } else if let Some((a, b)) = line.split_once(char::is_whitespace) {
            (a.trim(), b.trim())
        } else {
            return Err(DexError::Txt(format!("bad sparse-switch line: {line}")));
        };
        keys.push(parse_int(k)? as i32);
        targets.push(resolve_target(t, switch_base, labels)?);
    }
    let size = keys.len() as u16;
    let mut out = Vec::new();
    out.extend_from_slice(&0x0200u16.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    for k in &keys {
        out.extend_from_slice(&k.to_le_bytes());
    }
    for t in &targets {
        out.extend_from_slice(&t.to_le_bytes());
    }
    Ok(out)
}

fn resolve_target(
    tok: &str,
    switch_base: u32,
    labels: &HashMap<String, u32>,
) -> Result<i32, DexError> {
    let tok = tok.trim();
    // Raw relative already expressed vs switch.
    if let Some(rest) = tok.strip_prefix('+') {
        if let Ok(rel) = rest.parse::<i32>() {
            return Ok(rel);
        }
    }
    if let Some(rest) = tok.strip_prefix('-') {
        if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            if let Ok(rel) = rest.parse::<i32>() {
                return Ok(-rel);
            }
        }
    }
    let lab = if tok.starts_with(':') {
        tok.to_string()
    } else {
        format!(":{tok}")
    };
    let unit = labels
        .get(&lab)
        .or_else(|| labels.get(tok))
        .copied()
        .ok_or_else(|| DexError::Txt(format!("unknown switch target {tok}")))?;
    Ok(unit as i32 - switch_base as i32)
}

fn parse_int(s: &str) -> Result<i64, DexError> {
    let s = s.trim().trim_end_matches(',');
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        // Accept full-width unsigned hex (incl. sign-extended negatives like
        // 0xffffffffbd52c8c5 from older emitters) and re-interpret as i64 bits.
        return u64::from_str_radix(rest, 16)
            .map(|u| u as i64)
            .map_err(|e| DexError::Txt(format!("bad hex int {s}: {e}")));
    }
    s.parse::<i64>()
        .map_err(|e| DexError::Txt(format!("bad int {s}: {e}")))
}

/// Format raw payload bytes as mnemonic dex-txt block (without leading label).
pub fn format_payload_block(bytes: &[u8]) -> Result<String, DexError> {
    if bytes.len() < 4 {
        return Err(DexError::Txt("payload too short".into()));
    }
    let ident = u16::from_le_bytes([bytes[0], bytes[1]]);
    match ident {
        0x0100 => {
            let size = u16::from_le_bytes([bytes[2], bytes[3]]) as usize;
            if bytes.len() < 8 + size * 4 {
                return Err(DexError::Txt("truncated packed-switch".into()));
            }
            let first = i32::from_le_bytes(bytes[4..8].try_into().unwrap());
            let mut out = format!("    .packed-switch {first}\n");
            for i in 0..size {
                let off = 8 + i * 4;
                let rel = i32::from_le_bytes(bytes[off..off + 4].try_into().unwrap());
                // Emit as raw relative; emit.rs rewrites using switch offset when available.
                out.push_str(&format!("        +{rel}\n"));
            }
            out.push_str("    .end packed-switch\n");
            Ok(out)
        }
        0x0200 => {
            let size = u16::from_le_bytes([bytes[2], bytes[3]]) as usize;
            let need = 4 + size * 8;
            if bytes.len() < need {
                return Err(DexError::Txt("truncated sparse-switch".into()));
            }
            let mut out = String::from("    .sparse-switch\n");
            let keys_base = 4;
            let targets_base = 4 + size * 4;
            for i in 0..size {
                let key = i32::from_le_bytes(
                    bytes[keys_base + i * 4..keys_base + i * 4 + 4]
                        .try_into()
                        .unwrap(),
                );
                let rel = i32::from_le_bytes(
                    bytes[targets_base + i * 4..targets_base + i * 4 + 4]
                        .try_into()
                        .unwrap(),
                );
                out.push_str(&format!("        {key} -> +{rel}\n"));
            }
            out.push_str("    .end sparse-switch\n");
            Ok(out)
        }
        0x0300 => {
            if bytes.len() < 8 {
                return Err(DexError::Txt("truncated array-data".into()));
            }
            let width = u16::from_le_bytes([bytes[2], bytes[3]]) as usize;
            let size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
            let mut out = format!("    .array-data {width}\n");
            let mut pos = 8usize;
            for _ in 0..size {
                if pos + width > bytes.len() {
                    break;
                }
                let v = match width {
                    1 => bytes[pos] as i64,
                    2 => i16::from_le_bytes(bytes[pos..pos + 2].try_into().unwrap()) as i64,
                    4 => i32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as i64,
                    8 => i64::from_le_bytes(bytes[pos..pos + 8].try_into().unwrap()),
                    _ => 0,
                };
                let hex = match width {
                    1 => format!("0x{:x}", v as u8),
                    2 => format!("0x{:x}", v as u16),
                    4 => format!("0x{:x}", v as u32),
                    8 => format!("0x{:x}", v as u64),
                    _ => format!("0x{v:x}"),
                };
                out.push_str(&format!("        {hex}\n"));
                pos += width;
            }
            out.push_str("    .end array-data\n");
            Ok(out)
        }
        _ => Err(DexError::Txt(format!("unknown payload ident 0x{ident:04x}"))),
    }
}

/// Rewrite `+rel` targets in a packed/sparse block to absolute labels given switch byte offset
/// and the set of valid instruction byte offsets in the method.
pub fn rewrite_payload_targets_to_labels(
    block: &str,
    switch_byte_off: u32,
    valid_offsets: &std::collections::HashSet<u32>,
) -> String {
    let switch_units = switch_byte_off / 2;
    let mut out = String::new();
    for line in block.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('+') {
            if let Ok(rel) = rest.parse::<i32>() {
                let target_bytes = (switch_units as i32 + rel) * 2;
                if target_bytes >= 0 {
                    let tb = target_bytes as u32;
                    if valid_offsets.contains(&tb) {
                        out.push_str(&format!("        :L_{tb:08x}\n"));
                    } else {
                        // Keep relative — no insn starts at this offset.
                        out.push_str(&format!("        +{rel}\n"));
                    }
                    continue;
                }
            }
        }
        if let Some((k, rest)) = t.split_once("->") {
            let rest = rest.trim();
            if let Some(r) = rest.strip_prefix('+') {
                if let Ok(rel) = r.parse::<i32>() {
                    let target_bytes = (switch_units as i32 + rel) * 2;
                    if target_bytes >= 0 {
                        let tb = target_bytes as u32;
                        if valid_offsets.contains(&tb) {
                            out.push_str(&format!(
                                "        {} -> :L_{tb:08x}\n",
                                k.trim()
                            ));
                        } else {
                            out.push_str(&format!("        {} -> +{rel}\n", k.trim()));
                        }
                        continue;
                    }
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}
