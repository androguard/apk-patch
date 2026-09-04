//! dex-parser backed reference resolver for dex-bytecode.

use dex_bytecode::{RefKind, ResolveRef};
use dex_parser::DexFile;

pub struct DexResolver<'a> {
    dex: &'a DexFile,
}

impl<'a> DexResolver<'a> {
    pub fn new(dex: &'a DexFile) -> Self {
        Self { dex }
    }
}

impl ResolveRef for DexResolver<'_> {
    fn resolve(&self, kind: RefKind, index: u32) -> Option<String> {
        match kind {
            RefKind::String => self.dex.get_string(index).ok(),
            RefKind::Type => self.dex.get_type(index).ok(),
            RefKind::Field => {
                let f = self.dex.get_field_info(index).ok()?;
                Some(format!("{}->{}:{}", f.class, f.name, f.typ))
            }
            RefKind::Method => {
                let m = self.dex.get_method_info(index).ok()?;
                let mut proto = String::from("(");
                for p in &m.params {
                    proto.push_str(p);
                }
                proto.push(')');
                proto.push_str(&m.return_type);
                Some(format!("{}->{}{}", m.class, m.name, proto))
            }
            RefKind::MethodProto => {
                self.dex
                    .protos
                    .get_proto(
                        &self.dex.data,
                        &self.dex.types,
                        &self.dex.strings,
                        index,
                    )
                    .ok()
                    .map(|(ret, params)| {
                        let mut proto = String::from("(");
                        for p in params {
                            proto.push_str(&p);
                        }
                        proto.push(')');
                        proto.push_str(&ret);
                        proto
                    })
            }
            _ => None,
        }
    }
}
