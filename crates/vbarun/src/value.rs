use crate::{Error, Result};

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    #[default]
    Empty,
    Null,
    Missing,
    Integer(i16),
    Long(i32),
    LongLong(i64),
    Single(f32),
    Double(f64),
    /// Scaled by 10,000, independent of binary floating-point storage.
    Currency(i64),
    Date(f64),
    String(String),
    Boolean(bool),
    Byte(u8),
    Error(i32),
    Array(Array),
    Object(Option<crate::Object>),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Array {
    pub bounds: Vec<(i32, i32)>,
    pub values: Vec<Value>,
    pub element_type: String,
    pub dynamic: bool,
}
impl Array {
    pub fn new(bounds: Vec<(i32, i32)>, element_type: String, dynamic: bool, cap: usize) -> Result<Self> {
        let mut n = 1usize;
        for (lo, hi) in &bounds {
            let len = i64::from(*hi) - i64::from(*lo) + 1;
            if len <= 0 {
                return Err(Error::runtime(9, "Subscript out of range"));
            }
            n = n.checked_mul(len as usize).filter(|v| *v <= cap).ok_or_else(|| Error::limit("array elements"))?;
        }
        if bounds.is_empty() {
            n = 0;
        }
        let initial = default_value(&element_type);
        Ok(Self { bounds, values: vec![initial; n], element_type, dynamic })
    }
    pub fn offset(&self, indices: &[i32]) -> Result<usize> {
        if indices.len() != self.bounds.len() {
            return Err(Error::runtime(9, "Subscript out of range"));
        }
        let mut offset = 0usize;
        for ((lo, hi), i) in self.bounds.iter().zip(indices) {
            if i < lo || i > hi {
                return Err(Error::runtime(9, "Subscript out of range"));
            }
            let len = (i64::from(*hi) - i64::from(*lo) + 1) as usize;
            offset = offset
                .checked_mul(len)
                .and_then(|n| n.checked_add((i64::from(*i) - i64::from(*lo)) as usize))
                .ok_or_else(|| Error::runtime(9, "Subscript out of range"))?;
        }
        if offset >= self.values.len() {
            return Err(Error::runtime(9, "Subscript out of range"));
        }
        Ok(offset)
    }
}
pub(crate) fn mismatch() -> Error {
    Error::runtime(13, "Type mismatch")
}
pub(crate) fn overflow() -> Error {
    Error::runtime(6, "Overflow")
}
pub(crate) fn default_value(ty: &str) -> Value {
    match ty.to_ascii_lowercase().as_str() {
        "variant" | "" => Value::Empty,
        "integer" => Value::Integer(0),
        "long" => Value::Long(0),
        "longlong" => Value::LongLong(0),
        "single" => Value::Single(0.0),
        "double" => Value::Double(0.0),
        "currency" => Value::Currency(0),
        "date" => Value::Date(0.0),
        "string" => Value::String(String::new()),
        "boolean" => Value::Boolean(false),
        "byte" => Value::Byte(0),
        _ => Value::Object(None),
    }
}
impl Value {
    pub fn number(&self) -> Result<f64> {
        let n = match self {
            Self::Empty => 0.0,
            Self::Integer(n) => f64::from(*n),
            Self::Long(n) => f64::from(*n),
            Self::LongLong(n) => *n as f64,
            Self::Byte(n) => f64::from(*n),
            Self::Single(n) => f64::from(*n),
            Self::Double(n) | Self::Date(n) => *n,
            Self::Currency(n) => *n as f64 / 10000.0,
            Self::Boolean(b) => {
                if *b {
                    -1.0
                } else {
                    0.0
                }
            }
            Self::String(s) => s.trim().parse().map_err(|_| mismatch())?,
            Self::Null => return Err(Error::runtime(94, "Invalid use of Null")),
            _ => return Err(mismatch()),
        };
        if n.is_finite() { Ok(n) } else { Err(overflow()) }
    }
    pub fn text(&self) -> Result<String> {
        Ok(match self {
            Self::String(s) => s.clone(),
            Self::LongLong(n) => n.to_string(),
            Self::Currency(n) => {
                let value = n.unsigned_abs();
                let fraction = format!("{:04}", value % 10000);
                let fraction = fraction.trim_end_matches('0');
                format!("{}{}{}{}", if *n < 0 { "-" } else { "" }, value / 10000, if fraction.is_empty() { "" } else { "." }, fraction)
            }
            Self::Empty => String::new(),
            Self::Boolean(b) => if *b { "True" } else { "False" }.into(),
            Self::Error(n) => format!("Error {n}"),
            Self::Null => return Err(Error::runtime(94, "Invalid use of Null")),
            _ => self.number()?.to_string(),
        })
    }
    pub fn truth(&self) -> Result<bool> {
        if *self == Self::Null {
            return Ok(false);
        }
        if let Self::String(s) = self {
            if s.eq_ignore_ascii_case("true") {
                return Ok(true);
            }
            if s.eq_ignore_ascii_case("false") {
                return Ok(false);
            }
        }
        Ok(self.number()? != 0.0)
    }
    pub fn integer(&self) -> Result<i64> {
        if let Self::LongLong(value) = self {
            return Ok(*value);
        }
        let n = self.number()?.round_ties_even();
        if !(-9223372036854775808.0..9223372036854775808.0).contains(&n) {
            return Err(overflow());
        }
        Ok(n as i64)
    }
    pub fn coerce(&self, ty: &str) -> Result<Self> {
        if ty.eq_ignore_ascii_case(self.type_name()) {
            return Ok(self.clone());
        }
        Ok(match ty.to_ascii_lowercase().as_str() {
            "" | "variant" => self.clone(),
            "string" => Self::String(self.text()?),
            "boolean" => Self::Boolean(self.truth()?),
            "byte" => Self::Byte(u8::try_from(self.integer()?).map_err(|_| overflow())?),
            "integer" => Self::Integer(i16::try_from(self.integer()?).map_err(|_| overflow())?),
            "long" => Self::Long(i32::try_from(self.integer()?).map_err(|_| overflow())?),
            "longlong" => Self::LongLong(self.integer()?),
            "double" => Self::Double(self.number()?),
            "single" => {
                let n = self.number()? as f32;
                if !n.is_finite() {
                    return Err(overflow());
                }
                Self::Single(n)
            }
            "currency" => {
                let v = Self::Double(self.number()? * 10000.0).integer()?;
                Self::Currency(v)
            }
            "date" => {
                let n = crate::intrinsic::date_value(self)?;
                if !(-657434.0..2958466.0).contains(&n) {
                    return Err(overflow());
                }
                Self::Date(n)
            }
            _ => {
                if matches!(self, Self::Object(_)) {
                    self.clone()
                } else {
                    return Err(mismatch());
                }
            }
        })
    }
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Empty => "Empty",
            Self::Null => "Null",
            Self::Missing => "Error",
            Self::Integer(_) => "Integer",
            Self::Long(_) => "Long",
            Self::LongLong(_) => "LongLong",
            Self::Single(_) => "Single",
            Self::Double(_) => "Double",
            Self::Currency(_) => "Currency",
            Self::Date(_) => "Date",
            Self::String(_) => "String",
            Self::Boolean(_) => "Boolean",
            Self::Byte(_) => "Byte",
            Self::Error(_) => "Error",
            Self::Array(a) => match a.element_type.as_str() {
                "integer" => "Integer()",
                "long" => "Long()",
                "longlong" => "LongLong()",
                "single" => "Single()",
                "double" => "Double()",
                "currency" => "Currency()",
                "date" => "Date()",
                "string" => "String()",
                "boolean" => "Boolean()",
                "byte" => "Byte()",
                "variant" => "Variant()",
                _ => "Object()",
            },
            Self::Object(None) => "Nothing",
            Self::Object(Some(_)) => "Object",
        }
    }
    pub fn var_type(&self) -> i32 {
        match self {
            Self::Empty => 0,
            Self::Null => 1,
            Self::Integer(_) => 2,
            Self::Long(_) => 3,
            Self::Single(_) => 4,
            Self::Double(_) => 5,
            Self::Currency(_) => 6,
            Self::Date(_) => 7,
            Self::String(_) => 8,
            Self::Object(_) => 9,
            Self::Error(_) | Self::Missing => 10,
            Self::Boolean(_) => 11,
            Self::Byte(_) => 17,
            Self::LongLong(_) => 20,
            Self::Array(a) => 8192 + if a.element_type == "variant" { 12 } else { default_value(&a.element_type).var_type() },
        }
    }
    pub fn memory(&self) -> usize {
        match self {
            Self::String(s) => 32usize.saturating_add(s.len()),
            Self::Array(a) => a.values.iter().fold(64usize, |n, v| n.saturating_add(v.memory())),
            _ => 32,
        }
    }
}
