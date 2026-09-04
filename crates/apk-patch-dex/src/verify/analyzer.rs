//! Baksmali-style forward register-type dataflow (MethodAnalyzer / Dalvik verifier subset).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::access::{parse_access, proto_ins_words};
use crate::parse::{DexTxtInsn, DexTxtMethod};
use crate::DexError;

use super::regtype::{Category, RegType};

#[derive(Clone, Debug)]
struct Frame {
    regs: Vec<RegType>,
    /// Pending move-result type after an invoke / filled-new-array.
    pending_result: Option<RegType>,
}

impl Frame {
    fn new(n: usize) -> Self {
        Self {
            regs: vec![RegType::cat(Category::Uninit); n],
            pending_result: None,
        }
    }

    fn merge(&self, other: &Self) -> Self {
        let n = self.regs.len().max(other.regs.len());
        let mut regs = Vec::with_capacity(n);
        for i in 0..n {
            let a = self
                .regs
                .get(i)
                .cloned()
                .unwrap_or_else(|| RegType::cat(Category::Unknown));
            let b = other
                .regs
                .get(i)
                .cloned()
                .unwrap_or_else(|| RegType::cat(Category::Unknown));
            regs.push(a.merge(&b));
        }
        let pending_result = match (&self.pending_result, &other.pending_result) {
            (Some(a), Some(b)) => Some(a.merge(b)),
            (None, None) => None,
            _ => None, // conflict on pending → drop (must re-invoke)
        };
        Self {
            regs,
            pending_result,
        }
    }

    fn set(&mut self, idx: u16, ty: RegType) {
        if let Some(slot) = self.regs.get_mut(idx as usize) {
            *slot = ty;
        }
    }

    fn get(&self, idx: u16) -> RegType {
        self.regs
            .get(idx as usize)
            .cloned()
            .unwrap_or_else(|| RegType::cat(Category::Unknown))
    }

    fn set_wide(&mut self, idx: u16, lo: RegType) {
        let hi = lo
            .wide_hi_pair()
            .unwrap_or_else(|| RegType::cat(Category::Conflicted));
        self.set(idx, lo);
        self.set(idx + 1, hi);
    }

    fn invalidate_wide_overlap(&mut self, idx: u16) {
        // Writing a single-width into the high half of a wide pair, or into lo, breaks pairs.
        if idx > 0 {
            let prev = self.get(idx - 1);
            if prev.category.is_wide_lo() {
                self.set(idx - 1, RegType::cat(Category::Conflicted));
            }
        }
        let cur = self.get(idx);
        if cur.category.is_wide_lo() {
            self.set(idx + 1, RegType::cat(Category::Conflicted));
        }
        if cur.category.is_wide_hi() {
            self.set(idx - 1, RegType::cat(Category::Conflicted));
        }
    }
}

/// Run structural + register-type verification for one method.
pub fn analyze_method(
    method: &DexTxtMethod,
    class_descriptor: &str,
) -> Result<(), DexError> {
    let access = parse_access(&method.access);
    let is_static = access & 0x0008 != 0;
    let is_constructor = method.name == "<init>" || access & 0x10000 != 0;
    // Abstract / native / empty bodies need no register-type analysis.
    if method.insns.is_empty() {
        return Ok(());
    }
    let registers = match method.registers {
        Some(n) => n as usize,
        None => {
            return Err(DexError::Txt(format!(
                "method {}{} missing .registers",
                method.name, method.proto
            )));
        }
    };
    // `.registers 0` is valid for static no-arg methods with no locals.

    let (params, ret) = parse_proto(&method.proto);
    let ins = proto_ins_words(&method.proto, is_static) as usize;
    if registers < ins {
        return Err(DexError::Txt(format!(
            "{}{}: .registers {registers} < ins_size {ins}",
            method.name, method.proto
        )));
    }

    // Build label → insn index
    let mut labels: HashMap<String, usize> = HashMap::new();
    for (i, insn) in method.insns.iter().enumerate() {
        if let Some(ref lab) = insn.label {
            labels.insert(lab.clone(), i);
        }
    }

    // CFG successors
    let succs = build_successors(method, &labels)?;

    // Initial frame from signature
    let mut init = Frame::new(registers);
    let param_base = registers - ins;
    let mut p = param_base;
    if !is_static {
        if is_constructor {
            init.set(p as u16, RegType::uninit_this(class_descriptor));
        } else {
            init.set(p as u16, RegType::reference(class_descriptor));
        }
        p += 1;
    }
    for ty in &params {
        let rt = RegType::from_descriptor(ty);
        if rt.category.is_wide_lo() {
            init.set_wide(p as u16, rt);
            p += 2;
        } else {
            init.set(p as u16, rt);
            p += 1;
        }
    }

    let mut entry_frames: Vec<Option<Frame>> = vec![None; method.insns.len()];
    entry_frames[0] = Some(init);
    let mut work: VecDeque<usize> = VecDeque::new();
    work.push_back(0);
    let mut visited = vec![false; method.insns.len()];

    while let Some(idx) = work.pop_front() {
        let frame = entry_frames[idx]
            .clone()
            .ok_or_else(|| DexError::Txt(format!("internal: no frame at {idx}")))?;
        visited[idx] = true;

        let insn = &method.insns[idx];
        // Skip payload pseudo-ops for type checking (data only)
        if insn.mnemonic.starts_with('.') {
            for &s in &succs[idx] {
                merge_into(&mut entry_frames, s, &frame, &mut work);
            }
            continue;
        }

        let out = apply_insn(insn, &frame, &ret, &method.name, &method.proto)?;
        for &s in &succs[idx] {
            // Exception handlers get a Throwable in the first free sense: Dalvik puts exception
            // object in the same register state but we inject REFERENCE Throwable at join.
            let mut edge = out.clone();
            if is_handler_edge(method, &labels, idx, s) {
                // Handler entry: pending cleared; leave regs as-is (exception in move-exception)
                edge.pending_result = None;
            }
            merge_into(&mut entry_frames, s, &edge, &mut work);
        }
    }

    // Also verify catch handlers are reachable with merge from try bodies
    for c in &method.catches {
        let Some(&handler) = labels.get(&c.handler_label) else {
            continue;
        };
        let start = labels.get(&c.start_label).copied();
        let end = labels.get(&c.end_label).copied();
        if let (Some(start), Some(end)) = (start, end) {
            for i in start..end.min(method.insns.len()) {
                if let Some(ref fr) = entry_frames[i] {
                    let mut h = fr.clone();
                    h.pending_result = None;
                    merge_into(&mut entry_frames, handler, &h, &mut work);
                }
            }
        }
    }
    // Drain remaining work from exception merges
    while let Some(idx) = work.pop_front() {
        let frame = entry_frames[idx].clone().unwrap();
        let insn = &method.insns[idx];
        if insn.mnemonic.starts_with('.') {
            for &s in &succs[idx] {
                merge_into(&mut entry_frames, s, &frame, &mut work);
            }
            continue;
        }
        let out = apply_insn(insn, &frame, &ret, &method.name, &method.proto)?;
        for &s in &succs[idx] {
            merge_into(&mut entry_frames, s, &out, &mut work);
        }
    }

    let _ = visited;
    Ok(())
}

fn merge_into(
    entry_frames: &mut [Option<Frame>],
    idx: usize,
    incoming: &Frame,
    work: &mut VecDeque<usize>,
) {
    if idx >= entry_frames.len() {
        return;
    }
    match &entry_frames[idx] {
        None => {
            entry_frames[idx] = Some(incoming.clone());
            work.push_back(idx);
        }
        Some(old) => {
            let merged = old.merge(incoming);
            if !frames_eq(old, &merged) {
                entry_frames[idx] = Some(merged);
                work.push_back(idx);
            }
        }
    }
}

fn frames_eq(a: &Frame, b: &Frame) -> bool {
    a.pending_result == b.pending_result && a.regs == b.regs
}

fn is_handler_edge(
    method: &DexTxtMethod,
    labels: &HashMap<String, usize>,
    _from: usize,
    to: usize,
) -> bool {
    method.catches.iter().any(|c| labels.get(&c.handler_label) == Some(&to))
}

fn build_successors(
    method: &DexTxtMethod,
    labels: &HashMap<String, usize>,
) -> Result<Vec<Vec<usize>>, DexError> {
    let n = method.insns.len();
    let mut succs = vec![Vec::new(); n];
    for (i, insn) in method.insns.iter().enumerate() {
        if insn.mnemonic.starts_with('.') {
            if i + 1 < n {
                succs[i].push(i + 1);
            }
            continue;
        }
        let m = insn.mnemonic.as_str();
        if matches!(
            m,
            "return" | "return-void" | "return-wide" | "return-object" | "throw"
        ) {
            // no fallthrough
        } else if m == "goto" || m == "goto/16" || m == "goto/32" {
            if let Some(t) = first_label(&insn.operands) {
                let idx = *labels
                    .get(t)
                    .ok_or_else(|| DexError::Txt(format!("goto unknown label {t}")))?;
                succs[i].push(idx);
            }
        } else if m.starts_with("if-") {
            if let Some(t) = first_label(&insn.operands) {
                if let Some(&idx) = labels.get(t) {
                    succs[i].push(idx);
                }
            }
            if i + 1 < n {
                succs[i].push(i + 1);
            }
        } else if m == "packed-switch" || m == "sparse-switch" {
            // Fallthrough + all case targets from payload
            if i + 1 < n {
                succs[i].push(i + 1);
            }
            if let Some(payload_lab) = first_label(&insn.operands) {
                if let Some(&pidx) = labels.get(payload_lab) {
                    for t in payload_targets(&method.insns[pidx]) {
                        if let Some(&ti) = labels.get(t) {
                            succs[i].push(ti);
                        }
                    }
                }
            }
        } else {
            if i + 1 < n {
                succs[i].push(i + 1);
            }
        }
        // Unique
        let mut set = HashSet::new();
        succs[i].retain(|x| set.insert(*x));
    }
    Ok(succs)
}

fn payload_targets(insn: &DexTxtInsn) -> Vec<&str> {
    let mut out = Vec::new();
    for line in insn.operands.lines() {
        for tok in line.split_whitespace() {
            let tok = tok.trim().trim_end_matches(',');
            if tok.starts_with(':') {
                out.push(tok);
            }
        }
    }
    out
}

fn first_label(operands: &str) -> Option<&str> {
    for tok in operands.split(|c: char| c == ',' || c.is_whitespace()) {
        let tok = tok.trim().trim_end_matches(',');
        if tok.starts_with(':') {
            return Some(tok);
        }
    }
    None
}

fn apply_insn(
    insn: &DexTxtInsn,
    frame: &Frame,
    method_ret: &str,
    method_name: &str,
    method_proto: &str,
) -> Result<Frame, DexError> {
    let mut out = frame.clone();
    out.pending_result = None; // cleared unless we set a new one (invoke)
    let m = insn.mnemonic.as_str();
    let regs = parse_reg_operands(&insn.operands);
    let ctx = |msg: String| -> DexError {
        DexError::Txt(format!(
            "register-type error in {method_name}{method_proto}: {m}: {msg}"
        ))
    };

    // Helper: require register type
    let require = |frame: &Frame, idx: u16, expected: &RegType| -> Result<(), DexError> {
        let got = frame.get(idx);
        if !got.can_assign_to(expected) {
            return Err(ctx(format!(
                "v{idx} has type {got}, expected {expected}"
            )));
        }
        if expected.category.is_wide_lo() {
            let hi = frame.get(idx + 1);
            let expect_hi = expected
                .wide_hi_pair()
                .unwrap_or_else(|| RegType::cat(Category::LongHi));
            if !hi.can_assign_to(&expect_hi) {
                return Err(ctx(format!(
                    "v{idx} wide pair broken: v{} is {hi}, expected {expect_hi}",
                    idx + 1
                )));
            }
        }
        Ok(())
    };

    match m {
        "nop" => {}
        "move" | "move/from16" | "move/16" => {
            let (dst, src) = bin_regs(&regs)?;
            let src_ty = frame.get(src);
            if src_ty.category.is_wide() || src_ty.category == Category::Conflicted {
                return Err(ctx(format!("move from wide/conflicted {src_ty}")));
            }
            if src_ty.category == Category::Uninit || src_ty.category == Category::Unknown {
                return Err(ctx(format!("move from uninit v{src}")));
            }
            out.invalidate_wide_overlap(dst);
            out.set(dst, src_ty);
        }
        "move-wide" | "move-wide/from16" | "move-wide/16" => {
            let (dst, src) = bin_regs(&regs)?;
            let src_ty = frame.get(src);
            if !src_ty.category.is_wide_lo() {
                return Err(ctx(format!(
                    "move-wide source v{src} is {src_ty}, expected wide"
                )));
            }
            require(frame, src, &src_ty)?;
            out.invalidate_wide_overlap(dst);
            out.invalidate_wide_overlap(dst + 1);
            out.set_wide(dst, src_ty);
        }
        "move-object" | "move-object/from16" | "move-object/16" => {
            let (dst, src) = bin_regs(&regs)?;
            let src_ty = frame.get(src);
            if !src_ty.category.is_reference_like()
                || matches!(
                    src_ty.category,
                    Category::UninitRef | Category::UninitThis
                )
            {
                // allow UninitThis/Ref to move (smali does for this)
                if !matches!(
                    src_ty.category,
                    Category::Null
                        | Category::Reference
                        | Category::UninitRef
                        | Category::UninitThis
                ) {
                    return Err(ctx(format!(
                        "move-object source v{src} is {src_ty}"
                    )));
                }
            }
            out.invalidate_wide_overlap(dst);
            out.set(dst, src_ty);
        }
        "move-result" => {
            let dst = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            let Some(ty) = &frame.pending_result else {
                return Err(ctx("move-result without pending invoke result".into()));
            };
            if ty.category.is_wide() || ty.category == Category::Reference {
                return Err(ctx(format!("move-result type mismatch ({ty})")));
            }
            out.invalidate_wide_overlap(dst);
            out.set(dst, ty.clone());
        }
        "move-result-wide" => {
            let dst = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            let Some(ty) = &frame.pending_result else {
                return Err(ctx(
                    "move-result-wide without pending invoke result".into(),
                ));
            };
            if !ty.category.is_wide_lo() {
                return Err(ctx(format!("move-result-wide type mismatch ({ty})")));
            }
            out.set_wide(dst, ty.clone());
        }
        "move-result-object" => {
            let dst = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            let Some(ty) = &frame.pending_result else {
                return Err(ctx(
                    "move-result-object without pending invoke result".into(),
                ));
            };
            if !ty.category.is_reference_like() || ty.category.is_wide() {
                return Err(ctx(format!(
                    "move-result-object type mismatch ({ty})"
                )));
            }
            out.invalidate_wide_overlap(dst);
            out.set(dst, ty.clone());
        }
        "move-exception" => {
            let dst = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            out.invalidate_wide_overlap(dst);
            out.set(dst, RegType::reference("Ljava/lang/Throwable;"));
        }
        "return-void" => {
            if method_ret != "V" {
                return Err(ctx(format!(
                    "return-void in method returning {method_ret}"
                )));
            }
        }
        "return" => {
            let src = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            let expect = RegType::from_descriptor(method_ret);
            if expect.category.is_wide() || method_ret == "V" || method_ret.starts_with('L') || method_ret.starts_with('[')
            {
                return Err(ctx(format!("return used for {method_ret}")));
            }
            require(frame, src, &expect)?;
        }
        "return-wide" => {
            let src = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            if method_ret != "J" && method_ret != "D" {
                return Err(ctx(format!("return-wide in method returning {method_ret}")));
            }
            require(frame, src, &RegType::from_descriptor(method_ret))?;
        }
        "return-object" => {
            let src = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            if !method_ret.starts_with('L') && !method_ret.starts_with('[') {
                return Err(ctx(format!(
                    "return-object in method returning {method_ret}"
                )));
            }
            require(frame, src, &RegType::reference(method_ret))?;
        }
        "const/4" | "const/16" | "const" | "const/high16" => {
            let dst = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            let lit = parse_literal_from_ops(&insn.operands).unwrap_or(0);
            out.invalidate_wide_overlap(dst);
            out.set(dst, RegType::from_literal(lit));
        }
        "const-wide/16" | "const-wide/32" | "const-wide" | "const-wide/high16" => {
            let dst = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            out.set_wide(dst, RegType::cat(Category::LongLo));
        }
        "const-string" | "const-string/jumbo" => {
            let dst = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            out.invalidate_wide_overlap(dst);
            out.set(dst, RegType::reference("Ljava/lang/String;"));
        }
        "const-class" => {
            let dst = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            out.invalidate_wide_overlap(dst);
            out.set(dst, RegType::reference("Ljava/lang/Class;"));
        }
        "check-cast" => {
            let dst = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            require(frame, dst, &RegType::reference("Ljava/lang/Object;"))?;
            if let Some(ty) = type_ref_from_ops(&insn.operands) {
                out.set(dst, RegType::reference(ty));
            }
        }
        "instance-of" => {
            let dst = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            if let Some(src) = regs.get(1).copied() {
                require(frame, src, &RegType::reference("Ljava/lang/Object;"))?;
            }
            out.invalidate_wide_overlap(dst);
            out.set(dst, RegType::cat(Category::Boolean));
        }
        "array-length" => {
            let (dst, src) = bin_regs(&regs)?;
            let src_ty = frame.get(src);
            if !src_ty.category.is_reference_like()
                || src_ty
                    .type_desc
                    .as_ref()
                    .is_some_and(|t| !t.starts_with('['))
                    && src_ty.category != Category::Null
            {
                // Null OK; ref must be array if known
                if src_ty.category != Category::Null
                    && src_ty.type_desc.as_ref().is_some_and(|t| !t.starts_with('['))
                {
                    return Err(ctx(format!("array-length on non-array {src_ty}")));
                }
            }
            out.invalidate_wide_overlap(dst);
            out.set(dst, RegType::cat(Category::Integer));
        }
        "new-instance" => {
            let dst = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            let ty = type_ref_from_ops(&insn.operands)
                .ok_or_else(|| ctx("new-instance missing type".into()))?;
            out.invalidate_wide_overlap(dst);
            out.set(dst, RegType::uninit_ref(ty));
        }
        "new-array" => {
            let (dst, size) = bin_regs(&regs)?;
            require(frame, size, &RegType::cat(Category::Integer))?;
            let ty = type_ref_from_ops(&insn.operands)
                .ok_or_else(|| ctx("new-array missing type".into()))?;
            out.invalidate_wide_overlap(dst);
            out.set(dst, RegType::reference(ty));
        }
        "filled-new-array" | "filled-new-array/range" => {
            let ty = type_ref_from_ops(&insn.operands)
                .unwrap_or_else(|| "[Ljava/lang/Object;".into());
            out.pending_result = Some(RegType::reference(ty));
        }
        "throw" => {
            let src = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            require(frame, src, &RegType::reference("Ljava/lang/Throwable;"))?;
        }
        "goto" | "goto/16" | "goto/32" => {}
        x if x.starts_with("if-") => {
            // if-eqz / if-nez: int or reference
            // if-eq / if-ne: int or reference
            // if-lt/ge/gt/le: integers only
            if x.ends_with("z") {
                let src = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
                let ty = frame.get(src);
                if !(ty.category.is_integral() || ty.category.is_reference_like()) {
                    return Err(ctx(format!("if-*z on {ty}")));
                }
            } else if x == "if-eq" || x == "if-ne" {
                for &r in regs.iter().take(2) {
                    let ty = frame.get(r);
                    if !(ty.category.is_integral() || ty.category.is_reference_like()) {
                        return Err(ctx(format!("{x} on {ty}")));
                    }
                }
                // Both sides should be same category family (ref vs int)
                if regs.len() >= 2 {
                    let a = frame.get(regs[0]);
                    let b = frame.get(regs[1]);
                    let a_ref = a.category.is_reference_like();
                    let b_ref = b.category.is_reference_like();
                    if a_ref != b_ref
                        && a.category != Category::Null
                        && b.category != Category::Null
                    {
                        return Err(ctx(format!(
                            "{x} comparing incompatible {a} vs {b}"
                        )));
                    }
                }
            } else {
                for &r in regs.iter().take(2) {
                    require(frame, r, &RegType::cat(Category::Integer))?;
                }
            }
        }
        "aget" | "aget-boolean" | "aget-byte" | "aget-char" | "aget-short" => {
            let (dst, arr, idx) = tern_regs(&regs)?;
            require(frame, idx, &RegType::cat(Category::Integer))?;
            let arr_ty = frame.get(arr);
            if !arr_ty.can_assign_to(&RegType::reference("[I")) {
                return Err(ctx(format!("aget array v{arr} is {arr_ty}")));
            }
            out.invalidate_wide_overlap(dst);
            let elem = match m {
                "aget-boolean" => RegType::cat(Category::Boolean),
                "aget-byte" => RegType::cat(Category::Byte),
                "aget-char" => RegType::cat(Category::Char),
                "aget-short" => RegType::cat(Category::Short),
                _ => RegType::cat(Category::Integer),
            };
            out.set(dst, elem);
        }
        "aget-wide" => {
            let (dst, arr, idx) = tern_regs(&regs)?;
            require(frame, idx, &RegType::cat(Category::Integer))?;
            let arr_ty = frame.get(arr);
            if !arr_ty.can_assign_to(&RegType::reference("[J")) {
                return Err(ctx(format!("aget-wide array v{arr} is {arr_ty}")));
            }
            out.set_wide(dst, RegType::cat(Category::LongLo));
        }
        "aget-object" => {
            let (dst, arr, idx) = tern_regs(&regs)?;
            require(frame, idx, &RegType::cat(Category::Integer))?;
            require(frame, arr, &RegType::reference("[Ljava/lang/Object;"))?;
            let elem = frame
                .get(arr)
                .type_desc
                .as_ref()
                .and_then(|t| t.strip_prefix('['))
                .map(RegType::reference)
                .unwrap_or_else(|| RegType::reference("Ljava/lang/Object;"));
            out.invalidate_wide_overlap(dst);
            out.set(dst, elem);
        }
        "aput" | "aput-boolean" | "aput-byte" | "aput-char" | "aput-short" => {
            let (src, arr, idx) = tern_regs(&regs)?;
            require(frame, idx, &RegType::cat(Category::Integer))?;
            require(frame, arr, &RegType::reference("[I"))?;
            require(frame, src, &RegType::cat(Category::Integer))?;
        }
        "aput-wide" => {
            let (src, arr, idx) = tern_regs(&regs)?;
            require(frame, idx, &RegType::cat(Category::Integer))?;
            require(frame, arr, &RegType::reference("[J"))?;
            require(frame, src, &RegType::cat(Category::LongLo))?;
        }
        "aput-object" => {
            let (src, arr, idx) = tern_regs(&regs)?;
            require(frame, idx, &RegType::cat(Category::Integer))?;
            require(frame, arr, &RegType::reference("[Ljava/lang/Object;"))?;
            require(frame, src, &RegType::reference("Ljava/lang/Object;"))?;
        }
        x if x.starts_with("iget") || x.starts_with("sget") => {
            let dst = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            if x.starts_with("iget") {
                if let Some(obj) = regs.get(1).copied() {
                    let obj_ty = frame.get(obj);
                    if !obj_ty.category.is_reference_like() {
                        return Err(ctx(format!("iget object v{obj} is {obj_ty}")));
                    }
                }
            }
            let fty = field_type_from_ops(&insn.operands)
                .map(|t| RegType::from_descriptor(&t))
                .unwrap_or_else(|| RegType::cat(Category::Integer));
            if fty.category.is_wide_lo() {
                out.set_wide(dst, fty);
            } else {
                out.invalidate_wide_overlap(dst);
                out.set(dst, fty);
            }
        }
        x if x.starts_with("iput") || x.starts_with("sput") => {
            let src = regs.first().copied().ok_or_else(|| ctx("missing reg".into()))?;
            if x.starts_with("iput") {
                if let Some(obj) = regs.get(1).copied() {
                    let obj_ty = frame.get(obj);
                    // Dalvik allows UninitThis/UninitRef as the instance for field ops
                    // during construction (e.g. inner-class this$0 stores).
                    if !obj_ty.category.is_reference_like() {
                        return Err(ctx(format!("iput object v{obj} is {obj_ty}")));
                    }
                }
            }
            let fty = field_type_from_ops(&insn.operands)
                .map(|t| RegType::from_descriptor(&t))
                .unwrap_or_else(|| RegType::cat(Category::Integer));
            // Value may be UninitThis when storing `this` before <init> completes (rare);
            // normally require assignable — allow UninitThis/UninitRef as value too for
            // synthetic patterns.
            let src_ty = frame.get(src);
            if matches!(
                src_ty.category,
                Category::UninitThis | Category::UninitRef
            ) {
                // ok
            } else {
                require(frame, src, &fty)?;
            }
        }
        x if x.starts_with("invoke-") => {
            // Validate args loosely; set pending result from method return type
            let mref = method_ref_from_ops(&insn.operands);
            if let Some((_, _, proto)) = mref {
                let (params, ret) = parse_proto(&proto);
                // For invoke-direct <init>, initialize UninitRef/UninitThis
                if (x.starts_with("invoke-direct") || x.starts_with("invoke-super"))
                    && insn.operands.contains("-><init>(")
                {
                    if let Some(&this_r) = regs.first() {
                        let this_ty = frame.get(this_r);
                        if this_ty.category == Category::UninitRef
                            || this_ty.category == Category::UninitThis
                        {
                            let desc = this_ty
                                .type_desc
                                .clone()
                                .unwrap_or_else(|| "Ljava/lang/Object;".into());
                            out.set(this_r, RegType::reference(desc));
                        }
                    }
                }
                // Check arg count categories roughly
                let mut arg_regs = regs.clone();
                if x.contains("/range") {
                    // already expanded by parse_reg_operands
                }
                let mut ai = 0usize;
                // Non-static invokes: first reg is this
                let has_this = !x.contains("static") && !x.contains("custom");
                if has_this {
                    if let Some(&r) = arg_regs.first() {
                        let ty = frame.get(r);
                        if !ty.category.is_reference_like()
                            && ty.category != Category::UninitRef
                            && ty.category != Category::UninitThis
                        {
                            return Err(ctx(format!("invoke this-reg v{r} is {ty}")));
                        }
                        ai = 1;
                    }
                }
                for pty in &params {
                    let expect = RegType::from_descriptor(pty);
                    let Some(&r) = arg_regs.get(ai) else {
                        break;
                    };
                    require(frame, r, &expect)?;
                    ai += if expect.category.is_wide_lo() { 2 } else { 1 };
                    // For wide, parse_reg_operands lists only start regs for range;
                    // for non-range wide, smali lists one reg for the pair.
                    if !x.contains("/range") && expect.category.is_wide_lo() {
                        // single register listed for wide pair
                    }
                }
                let _ = ai;
                if ret != "V" {
                    out.pending_result = Some(RegType::from_descriptor(ret));
                } else {
                    out.pending_result = None;
                }
            } else {
                // Unknown ref form — still allow, no pending
                out.pending_result = frame.pending_result.clone();
            }
            // Restore: we cleared pending at start; invoke sets it above.
            // Also need to keep `out` register updates for <init>
        }
        x if x.starts_with("neg-")
            || x.starts_with("not-")
            || x.starts_with("int-to-")
            || x.starts_with("long-to-")
            || x.starts_with("float-to-")
            || x.starts_with("double-to-")
            || x == "int-to-long"
            || x == "int-to-float"
            || x == "int-to-double"
            || x == "long-to-int"
            || x == "long-to-float"
            || x == "long-to-double"
            || x == "float-to-int"
            || x == "float-to-long"
            || x == "float-to-double"
            || x == "double-to-int"
            || x == "double-to-long"
            || x == "double-to-float"
            || x == "int-to-byte"
            || x == "int-to-char"
            || x == "int-to-short" =>
        {
            apply_unary_or_binary_arith(m, &regs, frame, &mut out, &ctx)?;
        }
        x if x.contains("-int")
            || x.contains("-long")
            || x.contains("-float")
            || x.contains("-double")
            || x.starts_with("cmpl-")
            || x.starts_with("cmpg-")
            || x.starts_with("cmp-") =>
        {
            apply_unary_or_binary_arith(m, &regs, frame, &mut out, &ctx)?;
        }
        "packed-switch" | "sparse-switch" => {
            if let Some(&r) = regs.first() {
                require(frame, r, &RegType::cat(Category::Integer))?;
            }
        }
        "fill-array-data" => {
            if let Some(&r) = regs.first() {
                require(frame, r, &RegType::reference("[I"))?;
            }
        }
        "monitor-enter" | "monitor-exit" => {
            if let Some(&r) = regs.first() {
                require(frame, r, &RegType::reference("Ljava/lang/Object;"))?;
            }
        }
        _ => {
            // Conservative: don't invent types; leave frame (pending already cleared)
            // Re-copy register state without inventing
            out.regs = frame.regs.clone();
        }
    }

    // For invoke we already set pending on `out`; for others pending is None unless
    // we need to preserve from match arms that set it.
    if m.starts_with("invoke-") || m.starts_with("filled-new-array") {
        // pending_result already set in arm; but we zeroed at start then set in arm — OK
    }

    Ok(out)
}

fn apply_unary_or_binary_arith(
    m: &str,
    regs: &[u16],
    frame: &Frame,
    out: &mut Frame,
    ctx: &dyn Fn(String) -> DexError,
) -> Result<(), DexError> {
    let is_wide = m.contains("long") || m.contains("double") || m.contains("-wide");
    let is_float = m.contains("float") || m.contains("double");
    let dst_ty = if m.starts_with("cmp") {
        RegType::cat(Category::Integer)
    } else if m.contains("to-int") || m.ends_with("-to-byte") || m.ends_with("-to-char") || m.ends_with("-to-short")
    {
        RegType::cat(Category::Integer)
    } else if m.contains("to-long") {
        RegType::cat(Category::LongLo)
    } else if m.contains("to-float") {
        RegType::cat(Category::Float)
    } else if m.contains("to-double") {
        RegType::cat(Category::DoubleLo)
    } else if m.contains("long") {
        RegType::cat(Category::LongLo)
    } else if m.contains("double") {
        RegType::cat(Category::DoubleLo)
    } else if m.contains("float") {
        RegType::cat(Category::Float)
    } else {
        RegType::cat(Category::Integer)
    };

    let src_expect = if m.contains("long") && !m.contains("to-") {
        RegType::cat(Category::LongLo)
    } else if m.contains("double") && !m.contains("to-") {
        RegType::cat(Category::DoubleLo)
    } else if m.contains("float") && !m.contains("to-") && !m.contains("to-float") {
        RegType::cat(Category::Float)
    } else if m.contains("to-") {
        // source from prefix
        if m.starts_with("long-to") {
            RegType::cat(Category::LongLo)
        } else if m.starts_with("double-to") {
            RegType::cat(Category::DoubleLo)
        } else if m.starts_with("float-to") {
            RegType::cat(Category::Float)
        } else {
            RegType::cat(Category::Integer)
        }
    } else {
        RegType::cat(Category::Integer)
    };

    let dst = regs.first().copied().ok_or_else(|| ctx("missing dest".into()))?;
    // unary: neg-int vA, vB  or  int-to-long vA, vB
    if regs.len() >= 2 && (m.starts_with("neg-") || m.starts_with("not-") || m.contains("-to-")) {
        let src = regs[1];
        let got = frame.get(src);
        if !got.can_assign_to(&src_expect) {
            return Err(ctx(format!("v{src} is {got}, expected {src_expect}")));
        }
        if src_expect.category.is_wide_lo() {
            let hi = frame.get(src + 1);
            if !hi.category.is_wide_hi() {
                return Err(ctx(format!("wide source pair broken at v{src}")));
            }
        }
    } else if regs.len() >= 3 {
        // binary: add-int vA, vB, vC
        for &r in &regs[1..3] {
            let got = frame.get(r);
            if !got.can_assign_to(&src_expect) {
                return Err(ctx(format!("v{r} is {got}, expected {src_expect}")));
            }
        }
    } else if regs.len() == 2 && m.contains("/2addr") {
        let src = regs[1];
        let got = frame.get(src);
        if !got.can_assign_to(&src_expect) {
            return Err(ctx(format!("v{src} is {got}, expected {src_expect}")));
        }
    } else if regs.len() >= 2 && m.contains("/lit") {
        let src = regs[1];
        let got = frame.get(src);
        if !got.can_assign_to(&RegType::cat(Category::Integer)) {
            return Err(ctx(format!("v{src} is {got}, expected Integer")));
        }
    }

    let _ = (is_wide, is_float);
    if dst_ty.category.is_wide_lo() {
        out.set_wide(dst, dst_ty);
    } else {
        out.invalidate_wide_overlap(dst);
        out.set(dst, dst_ty);
    }
    Ok(())
}

fn parse_proto(proto: &str) -> (Vec<String>, &str) {
    let Some(rest) = proto.strip_prefix('(') else {
        return (vec![], "V");
    };
    let Some((params, ret)) = rest.split_once(')') else {
        return (vec![], "V");
    };
    let mut out = Vec::new();
    let mut chars = params.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            'L' => {
                let mut s = String::from("L");
                for ch in chars.by_ref() {
                    s.push(ch);
                    if ch == ';' {
                        break;
                    }
                }
                out.push(s);
            }
            '[' => {
                let mut s = String::from("[");
                while let Some('[') = chars.peek().copied() {
                    s.push(chars.next().unwrap());
                }
                if let Some('L') = chars.peek().copied() {
                    s.push(chars.next().unwrap());
                    for ch in chars.by_ref() {
                        s.push(ch);
                        if ch == ';' {
                            break;
                        }
                    }
                } else if let Some(p) = chars.next() {
                    s.push(p);
                }
                out.push(s);
            }
            _ => out.push(c.to_string()),
        }
    }
    (out, ret)
}

fn parse_reg_operands(operands: &str) -> Vec<u16> {
    let mut out = Vec::new();
    // Handle range: v0 ... v3 or v0 .. v3
    if operands.contains("...") || operands.contains("..") {
        let sep = if operands.contains("...") {
            "..."
        } else {
            ".."
        };
        if let Some((a, b)) = operands.split_once(sep) {
            let start = parse_one_reg(a).or_else(|| {
                a.split(|c: char| c == ',' || c == '{')
                    .rev()
                    .find_map(parse_one_reg)
            });
            let end = parse_one_reg(b).or_else(|| {
                b.split(|c: char| c == ',' || c == '}')
                    .find_map(parse_one_reg)
            });
            if let (Some(s), Some(e)) = (start, end) {
                for r in s..=e {
                    out.push(r);
                }
                return out;
            }
        }
    }
    for tok in operands.split(|c: char| c == ',' || c == '{' || c == '}' || c.is_whitespace()) {
        if let Some(r) = parse_one_reg(tok) {
            out.push(r);
        }
    }
    out
}

fn parse_one_reg(tok: &str) -> Option<u16> {
    let tok = tok.trim().trim_end_matches(',');
    let rest = tok.strip_prefix('v').or_else(|| tok.strip_prefix('p'))?;
    rest.parse().ok()
}

fn bin_regs(regs: &[u16]) -> Result<(u16, u16), DexError> {
    if regs.len() < 2 {
        return Err(DexError::Txt("expected 2 registers".into()));
    }
    Ok((regs[0], regs[1]))
}

fn tern_regs(regs: &[u16]) -> Result<(u16, u16, u16), DexError> {
    if regs.len() < 3 {
        return Err(DexError::Txt("expected 3 registers".into()));
    }
    Ok((regs[0], regs[1], regs[2]))
}

fn parse_literal_from_ops(operands: &str) -> Option<i64> {
    let last = operands
        .split(',')
        .last()?
        .trim()
        .split_whitespace()
        .next()?;
    if let Some(rest) = last.strip_prefix("0x").or_else(|| last.strip_prefix("0X")) {
        return i64::from_str_radix(rest, 16).ok();
    }
    last.parse().ok()
}

fn type_ref_from_ops(operands: &str) -> Option<String> {
    for tok in operands.split(|c: char| c == ',' || c.is_whitespace()) {
        let tok = tok.trim().trim_end_matches(',');
        if (tok.starts_with('L') && tok.ends_with(';')) || tok.starts_with('[') {
            return Some(tok.to_string());
        }
    }
    None
}

fn field_type_from_ops(operands: &str) -> Option<String> {
    for tok in operands.split(|c: char| c == ',' || c.is_whitespace()) {
        let tok = tok.trim().trim_end_matches(',');
        if let Some((_, rest)) = tok.split_once("->") {
            if let Some((_, ty)) = rest.split_once(':') {
                return Some(ty.to_string());
            }
        }
    }
    None
}

fn method_ref_from_ops(operands: &str) -> Option<(String, String, String)> {
    for tok in operands.split(|c: char| c == ',' || c.is_whitespace()) {
        let tok = tok.trim().trim_end_matches(',');
        if let Some((class, rest)) = tok.split_once("->") {
            if let Some((name, proto)) = rest.split_once('(') {
                return Some((
                    class.to_string(),
                    name.to_string(),
                    format!("({proto}"),
                ));
            }
        }
    }
    None
}
