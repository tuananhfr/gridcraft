use crate::{
    Argument, Clock, Error, Host, Result, Value,
    runtime::Interpreter,
    value::{Array, mismatch, overflow},
};
use gridcraft_core::date::{civil_from_days, days_from_civil};
const EPOCH: i64 = 25569;
const BLOCKED: &[&str] =
    &["shell", "kill", "open", "chdir", "chdrive", "curdir", "environ", "sendkeys", "appactivate", "declare", "filecopy", "mkdir", "rmdir", "dir"];
const FUNCTIONS: &[&str] = &[
    "len",
    "left",
    "right",
    "mid",
    "instr",
    "instrrev",
    "replace",
    "split",
    "join",
    "ucase",
    "lcase",
    "trim",
    "ltrim",
    "rtrim",
    "space",
    "string",
    "strcomp",
    "chr",
    "chrw",
    "asc",
    "ascw",
    "format",
    "strreverse",
    "abs",
    "atn",
    "sin",
    "cos",
    "tan",
    "exp",
    "log",
    "sqr",
    "int",
    "fix",
    "round",
    "rnd",
    "sgn",
    "now",
    "date",
    "time",
    "timer",
    "dateserial",
    "dateadd",
    "datediff",
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
    "weekday",
    "lbound",
    "ubound",
    "array",
    "isarray",
    "isnumeric",
    "isempty",
    "isnull",
    "isobject",
    "isdate",
    "ismissing",
    "typename",
    "vartype",
    "cint",
    "clng",
    "clnglng",
    "cdbl",
    "cstr",
    "cbool",
    "cdate",
    "cvar",
    "csng",
    "ccur",
    "cbyte",
    "cverr",
    "val",
    "iif",
    "choose",
    "switch",
];
pub(crate) fn exists(name: &str) -> bool {
    FUNCTIONS.contains(&name.trim_end_matches('$')) || BLOCKED.contains(&name)
}
struct Args<'a>(&'a [Argument]);
impl Args<'_> {
    fn v(&self, i: usize) -> Result<&Value> {
        self.0.get(i).map(|a| &a.value).filter(|v| **v != Value::Missing).ok_or_else(|| Error::runtime(449, "Argument not optional"))
    }
    fn n(&self, i: usize) -> Result<f64> {
        self.v(i)?.number()
    }
    fn i(&self, i: usize) -> Result<i64> {
        self.v(i)?.integer()
    }
    fn s(&self, i: usize) -> Result<String> {
        self.v(i)?.text()
    }
    fn opt_i(&self, i: usize, default: i64) -> Result<i64> {
        if self.0.get(i).is_none_or(|a| a.value == Value::Missing) { Ok(default) } else { self.i(i) }
    }
    fn opt_s(&self, i: usize, default: &str) -> Result<String> {
        if self.0.get(i).is_none_or(|a| a.value == Value::Missing) { Ok(default.into()) } else { self.s(i) }
    }
    fn nonnegative(&self, i: usize) -> Result<usize> {
        usize::try_from(self.i(i)?).map_err(|_| Error::runtime(5, "Invalid negative length"))
    }
}
pub(crate) fn call<H: Host, C: Clock>(vm: &mut Interpreter<'_, H, C>, name: &str, args: &[Argument]) -> Result<Value> {
    if BLOCKED.contains(&name) {
        return Err(Error::unsupported(name));
    }
    let a = Args(args);
    let compare_default = i64::from(vm.compare_text());
    let num = |n: f64| if n.is_finite() { Ok(Value::Double(n)) } else { Err(overflow()) };
    let string = |s| Ok(Value::String(s));
    let result = match name {
        "cint" => a.v(0)?.coerce("integer"),
        "clng" => a.v(0)?.coerce("long"),
        "clnglng" => a.v(0)?.coerce("longlong"),
        "cdbl" => a.v(0)?.coerce("double"),
        "cstr" => a.v(0)?.coerce("string"),
        "cbool" => a.v(0)?.coerce("boolean"),
        "cdate" => a.v(0)?.coerce("date"),
        "cvar" => Ok(a.v(0)?.clone()),
        "csng" => a.v(0)?.coerce("single"),
        "ccur" => a.v(0)?.coerce("currency"),
        "cbyte" => a.v(0)?.coerce("byte"),
        "cverr" => Ok(Value::Error(a.i(0)? as i32)),
        "len" => num(a.s(0)?.encode_utf16().count() as f64),
        "left" | "right" | "mid" => {
            let s = a.s(0)?;
            let units: Vec<u16> = s.encode_utf16().collect();
            let (start, count) = if name == "left" {
                (0, a.nonnegative(1)?)
            } else if name == "right" {
                let count = a.nonnegative(1)?.min(units.len());
                (units.len() - count, count)
            } else {
                let start = a.i(1)?;
                if start < 1 {
                    return Err(Error::runtime(5, "Invalid Mid start"));
                }
                let count = a.opt_i(2, units.len() as i64)?;
                if count < 0 {
                    return Err(Error::runtime(5, "Invalid Mid length"));
                }
                ((start - 1) as usize, count as usize)
            };
            string(String::from_utf16_lossy(units.get(start..start.saturating_add(count).min(units.len())).unwrap_or_default()))
        }
        "ucase" => string(a.s(0)?.to_uppercase()),
        "lcase" => string(a.s(0)?.to_lowercase()),
        "trim" => string(a.s(0)?.trim_matches(' ').into()),
        "ltrim" => string(a.s(0)?.trim_start_matches(' ').into()),
        "rtrim" => string(a.s(0)?.trim_end_matches(' ').into()),
        "space" | "string" => {
            let count = a.nonnegative(0)?;
            let s = if name == "space" {
                " ".into()
            } else if matches!(a.v(1)?, Value::String(_)) {
                a.s(1)?.chars().next().ok_or_else(|| Error::runtime(5, "Empty String character"))?.to_string()
            } else {
                char::from_u32(a.i(1)? as u32).ok_or_else(|| Error::runtime(5, "Invalid character"))?.to_string()
            };
            if count.checked_mul(s.len()).is_none_or(|n| n > vm.limits.max_string_length || n > vm.limits.max_memory_bytes) {
                return Err(Error::limit("string length"));
            }
            string(s.repeat(count))
        }
        "strreverse" => string(a.s(0)?.chars().rev().collect()),
        "chr" | "chrw" => {
            let n = a.i(0)?;
            if !(if name == "chr" { 0..=255 } else { -32768..=65535 }).contains(&n) {
                return Err(Error::runtime(5, "Invalid character code"));
            }
            if name == "chr" { string(ansi_character(n as u8).to_string()) } else { string(String::from_utf16_lossy(&[n as u16])) }
        }
        "asc" | "ascw" => {
            let s = a.s(0)?;
            let n = s.encode_utf16().next().ok_or_else(|| Error::runtime(5, "Empty Asc argument"))?;
            num(if name == "ascw" {
                f64::from(n as i16)
            } else {
                f64::from(ansi_code(s.chars().next().ok_or_else(mismatch)?).ok_or_else(|| Error::runtime(5, "Character is not in Windows-1252"))?)
            })
        }
        "strcomp" => {
            let (x, y) = (a.s(0)?, a.s(1)?);
            let text = a.opt_i(2, compare_default)? != 0;
            let cmp = if text { x.to_lowercase().cmp(&y.to_lowercase()) } else { x.cmp(&y) };
            num(if cmp.is_lt() {
                -1.0
            } else if cmp.is_gt() {
                1.0
            } else {
                0.0
            })
        }
        "instr" | "instrrev" => {
            let (start, hay, needle, compare) = if name == "instr" && args.len() > 2 {
                (a.i(0)?, a.s(1)?, a.s(2)?, a.opt_i(3, compare_default)?)
            } else {
                let hay = a.s(0)?;
                let start = if name == "instrrev" { a.opt_i(2, hay.encode_utf16().count() as i64)? } else { 1 };
                (start, hay, a.s(1)?, a.opt_i(3, compare_default)?)
            };
            let start = if name == "instrrev" && start == -1 { hay.encode_utf16().count() as i64 } else { start };
            if hay.is_empty() {
                return num(0.0);
            }
            if start < 1 {
                return Err(Error::runtime(5, "Invalid search start"));
            }
            let (hay, needle) = if compare != 0 { (hay.to_lowercase(), needle.to_lowercase()) } else { (hay, needle) };
            let units: Vec<u16> = hay.encode_utf16().collect();
            let find: Vec<u16> = needle.encode_utf16().collect();
            let mut pos = None;
            if find.is_empty() {
                pos = Some((start as usize).saturating_sub(1).min(units.len()));
            } else {
                for (i, window) in units.windows(find.len()).enumerate() {
                    if name == "instrrev" {
                        if i.saturating_add(find.len()) > start as usize {
                            break;
                        }
                    } else if i < (start - 1) as usize {
                        continue;
                    }
                    let mut matched = true;
                    for (a, b) in window.iter().zip(&find) {
                        vm.tick()?;
                        if a != b {
                            matched = false;
                            break;
                        }
                    }
                    if matched {
                        pos = Some(i);
                        if name != "instrrev" {
                            break;
                        }
                    }
                }
            }
            num(pos.map_or(0.0, |i| (i + 1) as f64))
        }
        "replace" => {
            let (s, find, replace) = (a.s(0)?, a.s(1)?, a.s(2)?);
            let start = a.opt_i(3, 1)?;
            if start < 1 {
                return Err(Error::runtime(5, "Invalid Replace start"));
            }
            let s: String = s.chars().skip((start - 1) as usize).collect();
            let count = a.opt_i(4, -1)?;
            if count < -1 {
                return Err(Error::runtime(5, "Invalid Replace count"));
            }
            let count = if count < 0 { usize::MAX } else { count as usize };
            let hits = if find.is_empty() { 0 } else { s.matches(&find).take(count).count() };
            if s.len().checked_add(hits.saturating_mul(replace.len())).is_none_or(|n| n > vm.limits.max_string_length) {
                return Err(Error::limit("string length"));
            }
            if a.opt_i(5, compare_default)? != 0 {
                let mut out = String::new();
                let mut rest = s.as_str();
                let mut used = 0usize;
                while !find.is_empty() && used < count {
                    vm.tick()?;
                    let Some((i, end)) = find_text(rest, &find) else {
                        break;
                    };
                    let Some(prefix) = rest.get(..i) else {
                        return Err(mismatch());
                    };
                    out.push_str(prefix);
                    out.push_str(&replace);
                    rest = rest.get(end..).ok_or_else(mismatch)?;
                    used += 1;
                    if out.len() > vm.limits.max_string_length {
                        return Err(Error::limit("string length"));
                    }
                }
                out.push_str(rest);
                string(out)
            } else {
                string(if find.is_empty() { s } else { s.replacen(&find, &replace, count) })
            }
        }
        "split" => {
            let s = a.s(0)?;
            let delimiter = a.opt_s(1, " ")?;
            let limit = a.opt_i(2, -1)?;
            if limit < -1 {
                return Err(Error::runtime(5, "Invalid Split limit"));
            }
            let cap = vm.array_cap();
            let count = if limit < 0 { cap.saturating_add(1) } else { (limit as usize).min(cap.saturating_add(1)) };
            let values: Vec<Value> = if s.is_empty() || count == 0 {
                Vec::new()
            } else if delimiter.is_empty() {
                vec![Value::String(s)]
            } else if a.opt_i(3, compare_default)? != 0 {
                let mut values = Vec::new();
                let mut rest = s.as_str();
                while values.len().saturating_add(1) < count {
                    vm.tick()?;
                    let Some((start, end)) = find_text(rest, &delimiter) else {
                        break;
                    };
                    values.push(Value::String(rest.get(..start).ok_or_else(mismatch)?.into()));
                    rest = rest.get(end..).ok_or_else(mismatch)?;
                }
                values.push(Value::String(rest.into()));
                values
            } else {
                s.splitn(count, &delimiter).map(|s| Value::String(s.into())).collect()
            };
            if values.len() > cap {
                return Err(Error::limit("array elements"));
            }
            Ok(Value::Array(Array { bounds: vec![(0, values.len() as i32 - 1)], values, element_type: "string".into(), dynamic: true }))
        }
        "join" => {
            let Value::Array(array) = a.v(0)? else {
                return Err(mismatch());
            };
            let sep = a.opt_s(1, " ")?;
            let mut out = String::new();
            for (i, v) in array.values.iter().enumerate() {
                vm.tick()?;
                let s = v.text()?;
                if out.len().saturating_add(s.len()).saturating_add(sep.len()) > vm.limits.max_string_length {
                    return Err(Error::limit("string length"));
                }
                if i > 0 {
                    out.push_str(&sep);
                }
                out.push_str(&s);
            }
            string(out)
        }
        "abs" => num(a.n(0)?.abs()),
        "atn" => num(a.n(0)?.atan()),
        "sin" => num(a.n(0)?.sin()),
        "cos" => num(a.n(0)?.cos()),
        "tan" => num(a.n(0)?.tan()),
        "exp" => num(a.n(0)?.exp()),
        "log" => {
            let n = a.n(0)?;
            if n <= 0.0 {
                return Err(Error::runtime(5, "Invalid Log argument"));
            }
            num(n.ln())
        }
        "sqr" => {
            let n = a.n(0)?;
            if n < 0.0 {
                return Err(Error::runtime(5, "Invalid Sqr argument"));
            }
            num(n.sqrt())
        }
        "int" => num(a.n(0)?.floor()),
        "fix" => num(a.n(0)?.trunc()),
        "sgn" => num(if a.n(0)? == 0.0 { 0.0 } else { a.n(0)?.signum() }),
        "round" => {
            let digits = a.opt_i(1, 0)?;
            if !(0..=28).contains(&digits) {
                return Err(Error::runtime(5, "Invalid Round precision"));
            }
            let scale = 10f64.powi(digits as i32);
            num((a.n(0)? * scale).round_ties_even() / scale)
        }
        "rnd" => {
            let n = if args.is_empty() { 1.0 } else { a.n(0)? };
            if n < 0.0 {
                vm.rng = (n as f32).to_bits();
            }
            if n != 0.0 {
                vm.rng = vm.rng.wrapping_mul(1140671485).wrapping_add(12820163) & 0xffffff;
                vm.last_random = f64::from(vm.rng) / 16777216.0;
            }
            num(vm.last_random)
        }
        "now" => Ok(Value::Date(vm.clock.date_serial())),
        "date" => Ok(Value::Date(vm.clock.date_serial().trunc())),
        "time" => Ok(Value::Date(vm.clock.date_serial().abs().fract())),
        "timer" => num(vm.clock.date_serial().abs().fract() * 86400.0),
        "dateserial" => Ok(Value::Date(date_serial(a.i(0)?, a.i(1)?, a.i(2)?)?)),
        "dateadd" => {
            let interval = a.s(0)?.to_ascii_lowercase();
            let count = a.i(1)?;
            let date = date_value(a.v(2)?)?;
            if !(-10_000_000..=10_000_000).contains(&count) {
                return Err(overflow());
            }
            if ["yyyy", "q", "m"].contains(&interval.as_str()) {
                let (y, m, d) = parts(date)?;
                let months = count
                    * if interval == "yyyy" {
                        12
                    } else if interval == "q" {
                        3
                    } else {
                        1
                    };
                let total = i64::from(y) * 12 + i64::from(m) - 1 + months;
                let year = total.div_euclid(12);
                let month = total.rem_euclid(12) + 1;
                let last = (date_serial(year, month + 1, 1)? - date_serial(year, month, 1)?) as i64;
                Ok(Value::Date(date_serial(year, month, i64::from(d).min(last))? + date.abs().fract()))
            } else {
                Ok(Value::Date(date + count as f64 * interval_days(&interval)?))
            }
        }
        "datediff" => {
            let interval = a.s(0)?.to_ascii_lowercase();
            let (x, y) = (date_value(a.v(1)?)?, date_value(a.v(2)?)?);
            let (xy, xm, _) = parts(x)?;
            let (yy, ym, _) = parts(y)?;
            let n = match interval.as_str() {
                "yyyy" => f64::from(yy - xy),
                "q" => f64::from((yy - xy) * 4) + f64::from((ym - 1) / 3) - f64::from((xm - 1) / 3),
                "m" => f64::from((yy - xy) * 12) + f64::from(ym) - f64::from(xm),
                "w" => ((y.trunc() - x.trunc()) / 7.0).trunc(),
                "ww" => {
                    let first = a.opt_i(3, 1)?;
                    if !(1..=7).contains(&first) {
                        return Err(Error::runtime(5, "Invalid first day"));
                    }
                    let offset = 7.0 - first as f64;
                    ((y.trunc() + offset) / 7.0).floor() - ((x.trunc() + offset) / 7.0).floor()
                }
                _ => (y / interval_days(&interval)?).floor() - (x / interval_days(&interval)?).floor(),
            };
            num(n)
        }
        "year" | "month" | "day" | "hour" | "minute" | "second" | "weekday" => {
            let date = date_value(a.v(0)?)?;
            let (y, m, d) = parts(date)?;
            let secs = (date.abs().fract() * 86400.0).round() as i64;
            num(match name {
                "year" => f64::from(y),
                "month" => f64::from(m),
                "day" => f64::from(d),
                "hour" => (secs / 3600 % 24) as f64,
                "minute" => (secs / 60 % 60) as f64,
                "second" => (secs % 60) as f64,
                _ => {
                    let first = a.opt_i(1, 1)?;
                    if !(1..=7).contains(&first) {
                        return Err(Error::runtime(5, "Invalid first day"));
                    }
                    ((date.trunc() as i64 + 6 - (first - 1)).rem_euclid(7) + 1) as f64
                }
            })
        }
        "array" => {
            if args.len() > vm.array_cap() {
                return Err(Error::limit("array elements"));
            }
            let base = vm.option_base();
            Ok(Value::Array(Array {
                bounds: vec![(base, base + args.len() as i32 - 1)],
                values: args.iter().map(|a| a.value.clone()).collect(),
                element_type: "variant".into(),
                dynamic: true,
            }))
        }
        "lbound" | "ubound" => {
            let Value::Array(array) = a.v(0)? else {
                return Err(mismatch());
            };
            let dim = a.opt_i(1, 1)?;
            let (lo, hi) = dim
                .checked_sub(1)
                .and_then(|n| usize::try_from(n).ok())
                .and_then(|i| array.bounds.get(i))
                .ok_or_else(|| Error::runtime(9, "Subscript out of range"))?;
            num(f64::from(if name == "lbound" { *lo } else { *hi }))
        }
        "isarray" => Ok(Value::Boolean(matches!(a.v(0)?, Value::Array(_)))),
        "isnumeric" => Ok(Value::Boolean(!matches!(a.v(0)?, Value::Date(_) | Value::Boolean(_)) && a.v(0)?.number().is_ok())),
        "isempty" => Ok(Value::Boolean(*a.v(0)? == Value::Empty)),
        "isnull" => Ok(Value::Boolean(*a.v(0)? == Value::Null)),
        "isobject" => Ok(Value::Boolean(matches!(a.v(0)?, Value::Object(_)))),
        "isdate" => Ok(Value::Boolean(date_value(a.v(0)?).is_ok())),
        "ismissing" => Ok(Value::Boolean(args.first().ok_or_else(|| Error::runtime(449, "Argument not optional"))?.value == Value::Missing)),
        "typename" => string(a.v(0)?.type_name().into()),
        "vartype" => Ok(Value::Long(a.v(0)?.var_type())),
        "iif" => Ok(if a.v(0)?.truth()? { a.v(1)? } else { a.v(2)? }.clone()),
        "choose" => {
            let i = a.i(0)?;
            Ok(usize::try_from(i).ok().and_then(|i| args.get(i)).map_or(Value::Null, |a| a.value.clone()))
        }
        "switch" => {
            if !args.len().is_multiple_of(2) {
                return Err(Error::runtime(450, "Switch requires condition/value pairs"));
            }
            let mut result = Value::Null;
            for [test, value] in args.as_chunks::<2>().0 {
                if test.value.truth()? {
                    result = value.value.clone();
                    break;
                }
            }
            Ok(result)
        }
        "val" => {
            let s: String = a.s(0)?.chars().filter(|c| !c.is_ascii_whitespace()).collect();
            if s.to_ascii_lowercase().starts_with("&h") {
                let digits: String = s.chars().skip(2).take_while(|c| c.is_ascii_hexdigit()).collect();
                num(i64::from_str_radix(&digits, 16).unwrap_or(0) as f64)
            } else {
                let mut end = 0;
                for (i, c) in s.char_indices() {
                    if c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-') {
                        end = i + c.len_utf8();
                    } else {
                        break;
                    }
                }
                let mut prefix = s.get(..end).unwrap_or_default();
                let n = loop {
                    if let Ok(n) = prefix.parse::<f64>() {
                        break n;
                    }
                    if prefix.is_empty() {
                        break 0.0;
                    }
                    prefix = prefix.get(..prefix.len() - 1).unwrap_or_default();
                };
                num(n)
            }
        }
        "format" => string(format_value(a.v(0)?, &a.opt_s(1, "")?)?),
        _ => Err(Error::runtime(35, format!("Function {name} not defined"))),
    }?;
    vm.validate(&result)?;
    Ok(result)
}
fn interval_days(interval: &str) -> Result<f64> {
    Ok(match interval {
        "y" | "d" | "w" => 1.0,
        "ww" => 7.0,
        "h" => 1.0 / 24.0,
        "n" => 1.0 / 1440.0,
        "s" => 1.0 / 86400.0,
        _ => return Err(Error::runtime(5, "Invalid date interval")),
    })
}
fn date_serial(year: i64, month: i64, day: i64) -> Result<f64> {
    if !(-100_000..=100_000).contains(&year) || !(-100_000..=100_000).contains(&month) || !(-10_000_000..=10_000_000).contains(&day) {
        return Err(overflow());
    }
    let year = if (0..100).contains(&year) { if year < 30 { 2000 + year } else { 1900 + year } } else { year };
    let y = year + (month - 1).div_euclid(12);
    let m = (month - 1).rem_euclid(12) + 1;
    let n = days_from_civil(y as i32, m as u32, 1) + day - 1 + EPOCH;
    if !(-657434..2958466).contains(&n) {
        return Err(overflow());
    }
    Ok(n as f64)
}
fn parts(date: f64) -> Result<(i32, u32, u32)> {
    if !date.is_finite() || !(-657434.0..2958466.0).contains(&date) {
        return Err(overflow());
    }
    Ok(civil_from_days(date.trunc() as i64 - EPOCH))
}
pub(crate) fn date_value(v: &Value) -> Result<f64> {
    if let Value::String(s) = v {
        if let Ok(n) = s.parse::<f64>() {
            parts(n)?;
            return Ok(n);
        }
        let mut split = s.split_whitespace();
        let date = split.next().ok_or_else(mismatch)?;
        let (date, time) = if date.contains(':') { (None, Some(date)) } else { (Some(date), split.next()) };
        let mut n = 0.0;
        if let Some(d) = date {
            let parts: Vec<_> = d.split(['-', '/']).collect();
            if parts.len() != 3 {
                return Err(mismatch());
            }
            let p = |i: usize| -> Result<i64> { parts.get(i).ok_or_else(mismatch)?.parse().map_err(|_| mismatch()) };
            n = if d.contains('-') { date_serial(p(0)?, p(1)?, p(2)?)? } else { date_serial(p(2)?, p(0)?, p(1)?)? };
        }
        if let Some(t) = time {
            let units: Vec<_> = t.split(':').collect();
            let h: u32 = units.first().ok_or_else(mismatch)?.parse().map_err(|_| mismatch())?;
            let m: u32 = units.get(1).ok_or_else(mismatch)?.parse().map_err(|_| mismatch())?;
            let s: u32 = units.get(2).map_or(Ok(0), |s| s.parse().map_err(|_| mismatch()))?;
            if h > 23 || m > 59 || s > 59 {
                return Err(mismatch());
            }
            n += f64::from(h * 3600 + m * 60 + s) / 86400.0;
        }
        Ok(n)
    } else {
        let n = v.number()?;
        parts(n)?;
        Ok(n)
    }
}
fn format_value(v: &Value, pattern: &str) -> Result<String> {
    if pattern.is_empty() {
        return v.text();
    }
    let p = pattern.to_ascii_lowercase();
    if !["currency", "standard", "fixed", "percent", "general number", "scientific"].contains(&p.as_str())
        && (p.contains('y')
            || p.contains('m')
            || p.contains('d')
            || p.contains('h')
            || p.contains('n')
            || p.contains('s')
            || ["short date", "long date", "short time", "long time", "general date"].contains(&p.as_str()))
    {
        let date = date_value(v)?;
        let (y, m, d) = parts(date)?;
        let seconds = (date.abs().fract() * 86400.0).round() as u32;
        let p = match p.as_str() {
            "short date" => "m/d/yyyy",
            "long date" => "dddd, mmmm d, yyyy",
            "general date" => "m/d/yyyy hh:nn:ss",
            "short time" => "hh:nn",
            "long time" => "hh:nn:ss",
            _ => &p,
        };
        let mut out = String::new();
        let twelve_hour = p.contains("am/pm");
        let normalized = p.replace("am/pm", "\u{1}");
        let mut chars = normalized.chars().peekable();
        let mut time_context = false;
        while let Some(c) = chars.next() {
            if c == '\u{1}' {
                out.push_str(if seconds / 3600 < 12 { "AM" } else { "PM" });
                continue;
            }
            if c == '"' {
                for literal in chars.by_ref() {
                    if literal == '"' {
                        break;
                    }
                    out.push(literal);
                }
                continue;
            }
            if c == '\\' {
                if let Some(literal) = chars.next() {
                    out.push(literal);
                }
                continue;
            }
            let mut n = 1;
            while chars.peek() == Some(&c) {
                chars.next();
                n += 1;
            }
            if c == 'h' {
                time_context = true;
            }
            if c == 'm' && n >= 3 && !time_context {
                let months =
                    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
                let month = months.get(m.saturating_sub(1) as usize).ok_or_else(mismatch)?;
                out.push_str(if n == 3 { month.get(..3).ok_or_else(mismatch)? } else { month });
                continue;
            }
            if c == 'd' && n >= 3 {
                let days = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
                let day = days.get((date.trunc() as i64 + 6).rem_euclid(7) as usize).ok_or_else(mismatch)?;
                out.push_str(if n == 3 { day.get(..3).ok_or_else(mismatch)? } else { day });
                continue;
            }
            let value = match c {
                'y' => Some(if n <= 2 { y.rem_euclid(100) as u32 } else { y as u32 }),
                'm' => Some(if time_context { seconds / 60 % 60 } else { m }),
                'd' => Some(d),
                'h' => Some(if twelve_hour {
                    let hour = seconds / 3600 % 12;
                    if hour == 0 { 12 } else { hour }
                } else {
                    seconds / 3600 % 24
                }),
                'n' => Some(seconds / 60 % 60),
                's' => Some(seconds % 60),
                _ => None,
            };
            if let Some(value) = value {
                if n > 1 {
                    out.push_str(&format!("{value:0width$}", width = n.min(4)));
                } else {
                    out.push_str(&value.to_string());
                }
            } else {
                for _ in 0..n {
                    out.push(c);
                }
            }
        }
        return Ok(out);
    }
    let n = v.number()?;
    if p == "general number" {
        return Ok(n.to_string());
    }
    if p == "scientific" {
        return Ok(format!("{n:.2E}"));
    }
    let p = match p.as_str() {
        "fixed" => "0.00",
        "standard" => "#,##0.00",
        "percent" => "0.00%",
        "currency" => "$#,##0.00",
        _ => &p,
    };
    let sections: Vec<_> = p.splitn(4, ';').collect();
    let section_index = if n < 0.0 && sections.len() > 1 {
        1
    } else if n == 0.0 && sections.len() > 2 {
        2
    } else {
        0
    };
    let section = sections.get(section_index).copied().unwrap_or(p);
    if !section.contains(['0', '#']) {
        return Ok(section.trim_matches('"').into());
    }
    let minimum_integer_digits = section.split('.').next().unwrap_or(section).chars().filter(|c| *c == '0').count().min(1000);
    let minimum_decimals = section.split_once('.').map_or(0, |(_, s)| s.chars().take_while(|c| matches!(c, '0' | '#')).filter(|c| *c == '0').count());
    let decimals = section.split_once('.').map_or(0, |(_, s)| s.chars().take_while(|c| matches!(c, '0' | '#')).count()).min(28);
    let n = if section_index == 1 { n.abs() } else { n } * if section.contains('%') { 100.0 } else { 1.0 };
    let mut out = format!("{n:.decimals$}");
    let whole_length = out.split('.').next().unwrap_or(&out).trim_start_matches('-').len();
    if minimum_integer_digits > whole_length {
        out.insert_str(usize::from(out.starts_with('-')), &"0".repeat(minimum_integer_digits - whole_length));
    }
    for _ in minimum_decimals..decimals {
        if out.ends_with('0') {
            out.pop();
        } else {
            break;
        }
    }
    if out.ends_with('.') {
        out.pop();
    }
    if section.contains(',') {
        let (whole, fraction) = out.split_once('.').unwrap_or((&out, ""));
        let negative = whole.starts_with('-');
        let digits = whole.trim_start_matches('-');
        let mut grouped = String::new();
        if negative {
            grouped.push('-');
        }
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                grouped.push(',');
            }
            grouped.push(c);
        }
        if !fraction.is_empty() {
            grouped.push('.');
            grouped.push_str(fraction);
        }
        out = grouped;
    }
    if section.contains('%') {
        out.push('%');
    }
    if section.contains('$') {
        out.insert(0, '$');
    }
    if section.contains('(') && section.contains(')') {
        out = format!("({out})");
    }
    Ok(out)
}
/// Iterative wildcard matching bounds work independently of recursive pattern shape.
pub(crate) fn like(text: &str, pattern: &str, compare_text: bool, mut tick: impl FnMut() -> Result<()>) -> Result<bool> {
    let (text, pattern) = if compare_text { (text.to_lowercase(), pattern.to_lowercase()) } else { (text.into(), pattern.into()) };
    let text: Vec<char> = text.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    let mut current = vec![false; text.len().saturating_add(1)];
    if let Some(v) = current.first_mut() {
        *v = true;
    }
    let mut i = 0usize;
    while let Some(c) = p.get(i).copied() {
        let mut next = vec![false; current.len()];
        let mut class = None;
        if c == '[' {
            let end = p
                .iter()
                .enumerate()
                .skip(i + 1)
                .find(|(_, c)| **c == ']')
                .map(|(i, _)| i)
                .ok_or_else(|| Error::runtime(93, "Invalid Like pattern"))?;
            class = p.get(i + 1..end);
            i = end;
        }
        for j in 0..=text.len() {
            tick()?;
            let before = current.get(j).copied().unwrap_or(false);
            let matched = if c == '*' {
                before || (j > 0 && next.get(j - 1).copied().unwrap_or(false))
            } else if j == 0 {
                false
            } else {
                let ch = text.get(j - 1).copied().unwrap_or('\0');
                current.get(j - 1).copied().unwrap_or(false)
                    && if c == '?' {
                        true
                    } else if c == '#' {
                        ch.is_ascii_digit()
                    } else if let Some(class) = class {
                        let negate = class.first() == Some(&'!');
                        let mut k = usize::from(negate);
                        let mut yes = false;
                        while let Some(lo) = class.get(k) {
                            tick()?;
                            if class.get(k + 1) == Some(&'-') {
                                if let Some(hi) = class.get(k + 2) {
                                    yes |= ch >= *lo && ch <= *hi;
                                    k += 3;
                                } else {
                                    return Err(Error::runtime(93, "Invalid Like range"));
                                }
                            } else {
                                yes |= ch == *lo;
                                k += 1;
                            }
                        }
                        yes != negate
                    } else {
                        ch == c
                    }
            };
            if let Some(v) = next.get_mut(j) {
                *v = matched;
            }
        }
        current = next;
        i += 1;
    }
    Ok(current.last().copied().unwrap_or(false))
}

fn find_text(text: &str, needle: &str) -> Option<(usize, usize)> {
    let folded = text.to_lowercase();
    let needle = needle.to_lowercase();
    let start = folded.find(&needle)?;
    let end = start.checked_add(needle.len())?;
    let mut offset = 0usize;
    let mut original_start = None;
    let mut original_end = None;
    for (index, ch) in text.char_indices() {
        if offset == start {
            original_start = Some(index);
        }
        if offset == end {
            original_end = Some(index);
            break;
        }
        offset = offset.saturating_add(ch.to_lowercase().map(char::len_utf8).sum::<usize>());
    }
    if original_end.is_none() && offset == end {
        original_end = Some(text.len());
    }
    Some((original_start?, original_end?))
}

const ANSI_SPECIAL: [char; 32] = [
    '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}',
    '\u{152}', '\u{8d}', '\u{17d}', '\u{8f}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '\u{9d}', '\u{17e}', '\u{178}',
];
fn ansi_character(code: u8) -> char {
    if (128..160).contains(&code) { ANSI_SPECIAL.get(usize::from(code - 128)).copied().unwrap_or('\u{fffd}') } else { char::from(code) }
}
fn ansi_code(ch: char) -> Option<u8> {
    ANSI_SPECIAL.iter().position(|c| *c == ch).map(|i| i as u8 + 128).or_else(|| u8::try_from(u32::from(ch)).ok().filter(|n| !(128..160).contains(n)))
}
