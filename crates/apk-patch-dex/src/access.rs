//! Dalvik access flag formatting and parsing.

pub fn format_access(flags: u32) -> String {
    let mut parts = Vec::new();
    if flags & 0x0001 != 0 {
        parts.push("public");
    }
    if flags & 0x0002 != 0 {
        parts.push("private");
    }
    if flags & 0x0004 != 0 {
        parts.push("protected");
    }
    if flags & 0x0008 != 0 {
        parts.push("static");
    }
    if flags & 0x0010 != 0 {
        parts.push("final");
    }
    if flags & 0x0040 != 0 {
        parts.push("volatile");
    }
    if flags & 0x0080 != 0 {
        parts.push("bridge");
    }
    if flags & 0x0100 != 0 {
        parts.push("native");
    }
    if flags & 0x0200 != 0 {
        parts.push("interface");
    }
    if flags & 0x0400 != 0 {
        parts.push("abstract");
    }
    if flags & 0x0800 != 0 {
        parts.push("strictfp");
    }
    if flags & 0x1000 != 0 {
        parts.push("synthetic");
    }
    if flags & 0x2000 != 0 {
        parts.push("annotation");
    }
    if flags & 0x4000 != 0 {
        parts.push("enum");
    }
    if flags & 0x10000 != 0 {
        parts.push("constructor");
    }
    if flags & 0x20000 != 0 {
        parts.push("declared-synchronized");
    }
    if parts.is_empty() {
        parts.push("package-private");
    }
    parts.join(" ")
}

pub fn parse_access(s: &str) -> u32 {
    let mut flags = 0u32;
    for part in s.split_whitespace() {
        match part {
            "public" => flags |= 0x0001,
            "private" => flags |= 0x0002,
            "protected" => flags |= 0x0004,
            "static" => flags |= 0x0008,
            "final" => flags |= 0x0010,
            "synchronized" => flags |= 0x0020,
            "volatile" => flags |= 0x0040,
            "bridge" | "transient" => flags |= 0x0080,
            "native" => flags |= 0x0100,
            "interface" => flags |= 0x0200,
            "abstract" => flags |= 0x0400,
            "strictfp" => flags |= 0x0800,
            "synthetic" => flags |= 0x1000,
            "annotation" => flags |= 0x2000,
            "enum" => flags |= 0x4000,
            "constructor" => flags |= 0x10000,
            "declared-synchronized" => flags |= 0x20000,
            "package-private" => {}
            _ => {}
        }
    }
    flags
}

pub fn format_proto(params: &[String], return_type: &str) -> String {
    let mut s = String::from("(");
    for p in params {
        s.push_str(p);
    }
    s.push(')');
    s.push_str(return_type);
    s
}

/// Count parameter words for `ins_size` (wide types take 2).
pub fn proto_ins_words(proto: &str, is_static: bool) -> u16 {
    let params = proto
        .strip_prefix('(')
        .and_then(|r| r.split_once(')'))
        .map(|(p, _)| p)
        .unwrap_or("");
    let mut words = if is_static { 0u16 } else { 1 };
    let mut chars = params.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            'L' => {
                words += 1;
                for ch in chars.by_ref() {
                    if ch == ';' {
                        break;
                    }
                }
            }
            '[' => {
                words += 1;
                while let Some('[') = chars.peek().copied() {
                    chars.next();
                }
                if let Some('L') = chars.peek().copied() {
                    chars.next();
                    for ch in chars.by_ref() {
                        if ch == ';' {
                            break;
                        }
                    }
                } else {
                    chars.next();
                }
            }
            'J' | 'D' => words += 2,
            _ => words += 1,
        }
    }
    words
}
