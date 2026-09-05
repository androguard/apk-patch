//! Register-type lattice matching smali/dexlib2 `RegisterType` categories.

use std::fmt;

/// Category codes aligned with `org.jf.dexlib2.analysis.RegisterType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Category {
    Unknown = 0,
    Uninit = 1,
    Null = 2,
    One = 3,
    Boolean = 4,
    Byte = 5,
    PosByte = 6,
    Short = 7,
    PosShort = 8,
    Char = 9,
    Integer = 10,
    Float = 11,
    LongLo = 12,
    LongHi = 13,
    DoubleLo = 14,
    DoubleHi = 15,
    UninitRef = 16,
    UninitThis = 17,
    Reference = 18,
    Conflicted = 19,
}

impl Category {
    pub const NAMES: [&'static str; 20] = [
        "Unknown",
        "Uninit",
        "Null",
        "One",
        "Boolean",
        "Byte",
        "PosByte",
        "Short",
        "PosShort",
        "Char",
        "Integer",
        "Float",
        "LongLo",
        "LongHi",
        "DoubleLo",
        "DoubleHi",
        "UninitRef",
        "UninitThis",
        "Reference",
        "Conflicted",
    ];

    pub fn is_wide_lo(self) -> bool {
        matches!(self, Category::LongLo | Category::DoubleLo)
    }

    pub fn is_wide_hi(self) -> bool {
        matches!(self, Category::LongHi | Category::DoubleHi)
    }

    pub fn is_wide(self) -> bool {
        self.is_wide_lo() || self.is_wide_hi()
    }

    pub fn is_reference_like(self) -> bool {
        matches!(
            self,
            Category::Null
                | Category::Reference
                | Category::UninitRef
                | Category::UninitThis
        )
    }

    fn from_u8(v: u8) -> Self {
        match v {
            0 => Category::Unknown,
            1 => Category::Uninit,
            2 => Category::Null,
            3 => Category::One,
            4 => Category::Boolean,
            5 => Category::Byte,
            6 => Category::PosByte,
            7 => Category::Short,
            8 => Category::PosShort,
            9 => Category::Char,
            10 => Category::Integer,
            11 => Category::Float,
            12 => Category::LongLo,
            13 => Category::LongHi,
            14 => Category::DoubleLo,
            15 => Category::DoubleHi,
            16 => Category::UninitRef,
            17 => Category::UninitThis,
            18 => Category::Reference,
            _ => Category::Conflicted,
        }
    }

    /// Primitive / integral categories assignable where an int-ish value is required.
    pub fn is_integral(self) -> bool {
        matches!(
            self,
            Category::Null
                | Category::One
                | Category::Boolean
                | Category::Byte
                | Category::PosByte
                | Category::Short
                | Category::PosShort
                | Category::Char
                | Category::Integer
        )
    }
}

/// Merge table copied from smali `RegisterType.mergeTable` (category × category → category).
const MERGE: [[u8; 20]; 20] = [
    // UNKNOWN
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19],
    // UNINIT
    [1, 1, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19],
    // NULL
    [2, 19, 2, 4, 4, 5, 6, 7, 8, 9, 10, 11, 19, 19, 19, 19, 19, 19, 18, 19],
    // ONE
    [3, 19, 4, 3, 4, 5, 6, 7, 8, 9, 10, 11, 19, 19, 19, 19, 19, 19, 19, 19],
    // BOOLEAN
    [4, 19, 4, 4, 4, 5, 6, 7, 8, 9, 10, 11, 19, 19, 19, 19, 19, 19, 19, 19],
    // BYTE
    [5, 19, 5, 5, 5, 5, 5, 7, 7, 10, 10, 11, 19, 19, 19, 19, 19, 19, 19, 19],
    // POS_BYTE
    [6, 19, 6, 6, 6, 5, 6, 7, 8, 9, 10, 11, 19, 19, 19, 19, 19, 19, 19, 19],
    // SHORT
    [7, 19, 7, 7, 7, 7, 7, 7, 7, 10, 10, 11, 19, 19, 19, 19, 19, 19, 19, 19],
    // POS_SHORT
    [8, 19, 8, 8, 8, 7, 8, 7, 8, 9, 10, 11, 19, 19, 19, 19, 19, 19, 19, 19],
    // CHAR
    [9, 19, 9, 9, 9, 10, 9, 10, 9, 9, 10, 11, 19, 19, 19, 19, 19, 19, 19, 19],
    // INTEGER
    [10, 19, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 19, 19, 19, 19, 19, 19, 19, 19],
    // FLOAT
    [11, 19, 11, 11, 11, 11, 11, 11, 11, 11, 10, 11, 19, 19, 19, 19, 19, 19, 19, 19],
    // LONG_LO
    [12, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 12, 19, 12, 19, 19, 19, 19, 19],
    // LONG_HI
    [13, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 13, 19, 13, 19, 19, 19, 19],
    // DOUBLE_LO
    [14, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 12, 19, 14, 19, 19, 19, 19, 19],
    // DOUBLE_HI
    [15, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 13, 19, 15, 19, 19, 19, 19],
    // UNINIT_REF — unique instances; merge only with Unknown
    [16, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19],
    // UNINIT_THIS
    [17, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 17, 19, 19],
    // REFERENCE
    [18, 19, 18, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 18, 19],
    // CONFLICTED
    [19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19, 19],
];

/// Inferred register type (category + optional reference descriptor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegType {
    pub category: Category,
    /// Present for Reference / UninitRef / UninitThis.
    pub type_desc: Option<String>,
    /// Allocation site id for UninitRef (so `<init>` initializes all aliases).
    pub uninit_id: Option<u32>,
}

impl fmt::Display for RegType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", Category::NAMES[self.category as usize])?;
        if let Some(ref t) = self.type_desc {
            write!(f, ",{t}")?;
        }
        if let Some(id) = self.uninit_id {
            write!(f, "#{id}")?;
        }
        Ok(())
    }
}

impl RegType {
    pub fn cat(c: Category) -> Self {
        Self {
            category: c,
            type_desc: None,
            uninit_id: None,
        }
    }

    pub fn reference(desc: impl Into<String>) -> Self {
        Self {
            category: Category::Reference,
            type_desc: Some(desc.into()),
            uninit_id: None,
        }
    }

    pub fn uninit_ref(desc: impl Into<String>, id: u32) -> Self {
        Self {
            category: Category::UninitRef,
            type_desc: Some(desc.into()),
            uninit_id: Some(id),
        }
    }

    pub fn uninit_this(desc: impl Into<String>) -> Self {
        Self {
            category: Category::UninitThis,
            type_desc: Some(desc.into()),
            uninit_id: None,
        }
    }

    pub fn from_descriptor(desc: &str) -> Self {
        match desc.chars().next().unwrap_or('V') {
            'Z' => Self::cat(Category::Boolean),
            'B' => Self::cat(Category::Byte),
            'S' => Self::cat(Category::Short),
            'C' => Self::cat(Category::Char),
            'I' => Self::cat(Category::Integer),
            'F' => Self::cat(Category::Float),
            'J' => Self::cat(Category::LongLo),
            'D' => Self::cat(Category::DoubleLo),
            'L' | '[' => Self::reference(desc),
            _ => Self::cat(Category::Conflicted),
        }
    }

    pub fn from_literal(v: i64) -> Self {
        if v < -32768 {
            Self::cat(Category::Integer)
        } else if v < -128 {
            Self::cat(Category::Short)
        } else if v < 0 {
            Self::cat(Category::Byte)
        } else if v == 0 {
            Self::cat(Category::Null)
        } else if v == 1 {
            Self::cat(Category::One)
        } else if v < 128 {
            Self::cat(Category::PosByte)
        } else if v < 32768 {
            Self::cat(Category::PosShort)
        } else if v < 65536 {
            Self::cat(Category::Char)
        } else {
            Self::cat(Category::Integer)
        }
    }

    pub fn wide_hi_pair(&self) -> Option<Self> {
        match self.category {
            Category::LongLo => Some(Self::cat(Category::LongHi)),
            Category::DoubleLo => Some(Self::cat(Category::DoubleHi)),
            _ => None,
        }
    }

    pub fn merge(&self, other: &Self) -> Self {
        if self == other {
            return self.clone();
        }
        // UninitRef: same allocation site merges; different sites conflict.
        if self.category == Category::UninitRef || other.category == Category::UninitRef {
            if self.category == Category::Unknown {
                return other.clone();
            }
            if other.category == Category::Unknown {
                return self.clone();
            }
            if self.category == Category::UninitRef
                && other.category == Category::UninitRef
                && self.uninit_id == other.uninit_id
                && self.type_desc == other.type_desc
            {
                return self.clone();
            }
            return Self::cat(Category::Conflicted);
        }
        let merged = MERGE[self.category as usize][other.category as usize];
        let cat = Category::from_u8(merged);
        if cat == Category::Reference {
            let desc = match (&self.type_desc, &other.type_desc) {
                (Some(x), Some(y)) if x == y => Some(x.clone()),
                (Some(x), Some(y)) => Some(common_super(x, y)),
                (Some(x), None) | (None, Some(x)) => Some(x.clone()),
                _ => Some("Ljava/lang/Object;".into()),
            };
            return Self {
                category: Category::Reference,
                type_desc: desc,
                uninit_id: None,
            };
        }
        if cat == Category::UninitThis {
            return self
                .type_desc
                .clone()
                .or_else(|| other.type_desc.clone())
                .map(Self::uninit_this)
                .unwrap_or_else(|| Self::cat(Category::UninitThis));
        }
        Self::cat(cat)
    }

    /// Can this value be used where `expected` is required?
    pub fn can_assign_to(&self, expected: &Self) -> bool {
        if self.category == Category::Unknown || self.category == Category::Uninit {
            return false;
        }
        if self.category == Category::Conflicted {
            return false;
        }
        match expected.category {
            Category::Integer => self.category.is_integral() || self.category == Category::Float,
            Category::Float => {
                self.category.is_integral()
                    || self.category == Category::Float
                    || self.category == Category::Integer
            }
            Category::Boolean => matches!(
                self.category,
                Category::Boolean | Category::One | Category::Null | Category::Integer
                    | Category::Byte | Category::PosByte
            ),
            Category::Byte => self.category.is_integral(),
            Category::Short => self.category.is_integral(),
            Category::Char => self.category.is_integral(),
            Category::LongLo => {
                self.category == Category::LongLo || self.category == Category::DoubleLo
            }
            Category::DoubleLo => {
                self.category == Category::DoubleLo || self.category == Category::LongLo
            }
            Category::LongHi => {
                self.category == Category::LongHi || self.category == Category::DoubleHi
            }
            Category::DoubleHi => {
                self.category == Category::DoubleHi || self.category == Category::LongHi
            }
            Category::Reference => {
                self.category.is_reference_like()
                    && self.category != Category::UninitRef
                    && self.category != Category::UninitThis
            }
            Category::UninitRef | Category::UninitThis => self.category == expected.category,
            Category::Null => self.category == Category::Null,
            Category::One => matches!(
                self.category,
                Category::One | Category::Boolean | Category::Integer | Category::PosByte
            ),
            _ => self.category == expected.category,
        }
    }
}

fn common_super(a: &str, b: &str) -> String {
    if a == b {
        return a.into();
    }
    if a.starts_with('[') && b.starts_with('[') {
        let ca = &a[1..];
        let cb = &b[1..];
        if ca.len() == 1 && ca == cb {
            return a.into();
        }
        if matches!(ca.chars().next(), Some('L' | '['))
            && matches!(cb.chars().next(), Some('L' | '['))
        {
            return format!("[{}", common_super(ca, cb));
        }
        return "Ljava/lang/Object;".into();
    }
    "Ljava/lang/Object;".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_byte_char_is_integer() {
        let m = RegType::cat(Category::Byte).merge(&RegType::cat(Category::Char));
        assert_eq!(m.category, Category::Integer);
    }

    #[test]
    fn merge_null_ref_is_ref() {
        let m = RegType::cat(Category::Null).merge(&RegType::reference("Ljava/lang/String;"));
        assert_eq!(m.category, Category::Reference);
        assert_eq!(m.type_desc.as_deref(), Some("Ljava/lang/String;"));
    }

    #[test]
    fn merge_int_ref_conflict() {
        let m = RegType::cat(Category::Integer).merge(&RegType::reference("Ljava/lang/Object;"));
        assert_eq!(m.category, Category::Conflicted);
    }

    #[test]
    fn literal_categories() {
        assert_eq!(RegType::from_literal(0).category, Category::Null);
        assert_eq!(RegType::from_literal(1).category, Category::One);
        assert_eq!(RegType::from_literal(100).category, Category::PosByte);
    }
}
