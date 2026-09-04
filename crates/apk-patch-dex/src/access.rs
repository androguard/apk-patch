//! Dalvik access flag formatting.

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
        parts.push("declared-synchronized");
    }
    if flags & 0x0200 != 0 {
        parts.push("native");
    }
    if flags & 0x0400 != 0 {
        parts.push("interface");
    }
    if flags & 0x0800 != 0 {
        parts.push("abstract");
    }
    if flags & 0x1000 != 0 {
        parts.push("strictfp");
    }
    if flags & 0x2000 != 0 {
        parts.push("synthetic");
    }
    if flags & 0x4000 != 0 {
        parts.push("annotation");
    }
    if flags & 0x8000 != 0 {
        parts.push("enum");
    }
    if parts.is_empty() {
        parts.push("package-private");
    }
    parts.join(" ")
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
