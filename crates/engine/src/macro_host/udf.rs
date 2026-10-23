//! Formula-only VBA capabilities. References become read-only Range objects; scalar coercion
//! reads their Value, preserving VBA's default-member semantics for untyped parameters.
use gridcraft_calc::eval::{Area, ExternalArg, ExternalError, ExternalFunctions, Host as CalcHost};
use gridcraft_core::{CellError, CellRef, RangeRef, Value as CellValue};
use gridcraft_vbarun::{Argument, Error, Host, Interpreter, Limits, Object, Value};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

const CELLS: u64 = 4096;
#[derive(Debug)]
pub(crate) struct Functions {
    project: Option<crate::macros::Analysis>,
}
impl Functions {
    pub fn new(wb: &gridcraft_model::Workbook) -> Self {
        Self { project: crate::macros::project(wb) }
    }
}
impl ExternalFunctions for Functions {
    fn resolves(&self, wb: &gridcraft_model::Workbook, name: &str) -> bool {
        wb.vba_project.is_some()
            && self
                .project
                .as_ref()
                .and_then(|p| p.as_ref().ok())
                .and_then(|p| gridcraft_vba::find_udf(p, crate::macros::function_name(name)))
                .is_some_and(|(_, p)| p.compatibility.level == 1)
    }
    fn call(&self, calc: &mut dyn CalcHost, sheet: usize, _at: CellRef, name: &str, args: &[ExternalArg]) -> Option<CellValue> {
        calc.workbook().vba_project.as_ref()?;
        let project = self.project.as_ref()?.as_ref().ok()?;
        let (module, procedure) = gridcraft_vba::find_udf(project, crate::macros::function_name(name))?;
        if procedure.compatibility.level != 1 {
            return None;
        }
        let result = (|| {
            let clock = super::RunClock::new();
            let program = compiled_program(project)?;
            let mut host = ReadHost { calc: &mut *calc, sheet, objects: Vec::new() };
            let mut arguments = Vec::with_capacity(args.len());
            let mut argument_bytes = 1024 * 1024;
            for (index, arg) in args.iter().enumerate() {
                let value = match arg {
                    ExternalArg::Value(value) => from_cell_bounded(value, &mut argument_bytes, 0)?,
                    ExternalArg::Range(area) => {
                        if area.range.count() > CELLS {
                            return Err(Error { fatal: true, ..Error::runtime(-32000, "VBA sandbox limit: range cells") });
                        }
                        let ty = procedure.params.get(index).and_then(|p| p.type_name.as_deref()).unwrap_or("variant").to_ascii_lowercase();
                        if matches!(ty.as_str(), "range" | "object" | "variant" | "") {
                            host.object(ExcelObject::Range(*area))?
                        } else {
                            host.values_bounded(*area, &mut argument_bytes)?
                        }
                    }
                };
                arguments.push(Argument::positional(value));
            }
            let limits = Limits {
                max_steps: 10_000,
                max_call_depth: 32,
                max_array_elements: CELLS as usize,
                max_string_length: 64 * 1024,
                max_memory_bytes: 1024 * 1024,
                deadline_ms: Some(50),
                ..Limits::default()
            };
            let mut interpreter = Interpreter::new(&program, host, clock, limits);
            let mut future = Box::pin(interpreter.run_async(&module.name, &procedure.name, &arguments));
            let mut context = Context::from_waker(Waker::noop());
            let result = match future.as_mut().poll(&mut context) {
                Poll::Ready(result) => result,
                Poll::Pending => Err(Error::runtime(1004, "A worksheet function cannot suspend")),
            };
            drop(future);
            let value = interpreter.host.scalar(result?)?;
            to_cell(value)
        })();
        Some(match result {
            Ok(value) => value,
            Err(error) => {
                let line = if error.module.is_empty() { procedure.line as usize } else { error.line };
                calc.external_error(ExternalError {
                    message: error.description,
                    module: if error.module.is_empty() { module.name.clone() } else { error.module },
                    line,
                    limit: error.fatal,
                });
                CellValue::Error(CellError::Value)
            }
        })
    }
}
type Compiled = gridcraft_vbarun::Result<std::sync::Arc<gridcraft_vbarun::Program>>;
thread_local! {
    static PROGRAM: std::cell::RefCell<Option<(std::sync::Arc<gridcraft_vba::Project>, Compiled)>> = const { std::cell::RefCell::new(None) };
}
fn compiled_program(project: &std::sync::Arc<gridcraft_vba::Project>) -> Compiled {
    PROGRAM.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((key, result)) = cache.as_ref()
            && std::sync::Arc::ptr_eq(key, project)
        {
            return result.clone();
        }
        let result = super::execution_program(project).map(std::sync::Arc::new);
        *cache = Some((project.clone(), result.clone()));
        result
    })
}
#[derive(Clone)]
enum ExcelObject {
    Application,
    Functions,
    Workbook,
    Dimension(Area, bool),
    Sheet(usize),
    Sheets,
    Range(Area),
    Global(String),
}
struct ReadHost<'a> {
    calc: &'a mut dyn CalcHost,
    sheet: usize,
    objects: Vec<ExcelObject>,
}
fn denied() -> Error {
    Error::runtime(1004, "Worksheet functions cannot change the workbook or request interaction")
}
fn invalid() -> Error {
    Error::runtime(1004, "Invalid worksheet function argument")
}
fn from_cell(value: &CellValue) -> gridcraft_vbarun::Result<Value> {
    from_cell_bounded(value, &mut (1024 * 1024), 0)
}
fn resource_limit() -> Error {
    Error { fatal: true, ..Error::runtime(-32000, "VBA sandbox limit: worksheet arguments") }
}
fn from_cell_bounded(value: &CellValue, bytes: &mut usize, depth: usize) -> gridcraft_vbarun::Result<Value> {
    if depth >= 32 {
        return Err(resource_limit());
    }
    let size = if let CellValue::Text(text) = value {
        if text.len() > 64 * 1024 {
            return Err(resource_limit());
        }
        text.len().saturating_add(32)
    } else {
        32
    };
    *bytes = bytes.checked_sub(size).ok_or_else(resource_limit)?;
    Ok(match value {
        CellValue::Empty => Value::Empty,
        CellValue::Number(n) => Value::Double(*n),
        CellValue::Bool(b) => Value::Boolean(*b),
        CellValue::Text(t) => Value::String(t.to_string()),
        CellValue::Error(_) => return Err(invalid()),
        CellValue::Array(a) => {
            if a.data.len() > CELLS as usize {
                return Err(resource_limit());
            }
            Value::Array(gridcraft_vbarun::Array {
                bounds: vec![(1, a.rows as i32), (1, a.cols as i32)],
                values: a.data.iter().map(|v| from_cell_bounded(v, bytes, depth + 1)).collect::<gridcraft_vbarun::Result<_>>()?,
                element_type: "variant".into(),
                dynamic: false,
            })
        }
    })
}
fn to_cell(value: Value) -> gridcraft_vbarun::Result<CellValue> {
    Ok(match value {
        Value::Empty => CellValue::Empty,
        Value::String(s) => CellValue::Text(s.into()),
        Value::Boolean(b) => CellValue::Bool(b),
        Value::Array(_) | Value::Object(_) | Value::Null | Value::Missing | Value::Error(_) => return Err(invalid()),
        v => CellValue::Number(v.number()?),
    })
}
impl ReadHost<'_> {
    fn object(&mut self, object: ExcelObject) -> gridcraft_vbarun::Result<Value> {
        if self.objects.len() >= CELLS as usize {
            return Err(resource_limit());
        }
        let id = self.objects.len() as u64;
        self.objects.push(object);
        Ok(Value::Object(Some(id)))
    }
    fn range(&mut self, sheet: usize, args: &[Argument]) -> gridcraft_vbarun::Result<Value> {
        let text = args.first().ok_or_else(invalid)?.value.text()?;
        let range = RangeRef::parse(&text).ok_or_else(invalid)?;
        if range.count() > CELLS {
            return Err(resource_limit());
        }
        self.object(ExcelObject::Range(Area { sheet, range }))
    }
    fn values(&mut self, area: Area) -> gridcraft_vbarun::Result<Value> {
        self.values_bounded(area, &mut (1024 * 1024))
    }
    fn values_bounded(&mut self, area: Area, bytes: &mut usize) -> gridcraft_vbarun::Result<Value> {
        if area.range.count() > CELLS {
            return Err(resource_limit());
        }
        if !self.calc.note_precedent(area) {
            return Err(Error { fatal: true, ..Error::runtime(-32000, "VBA sandbox limit: precedents") });
        }
        let values = area
            .range
            .iter()
            .map(|c| from_cell_bounded(&self.calc.cell_value(area.sheet, c), bytes, 0))
            .collect::<gridcraft_vbarun::Result<Vec<_>>>()?;
        if area.range.count() == 1 {
            return values.into_iter().next().ok_or_else(invalid);
        }
        Ok(Value::Array(gridcraft_vbarun::Array {
            bounds: vec![(1, area.range.height() as i32), (1, area.range.width() as i32)],
            values,
            element_type: "variant".into(),
            dynamic: false,
        }))
    }
}
impl Host for ReadHost<'_> {
    fn global(&mut self, name: &str) -> Option<Value> {
        let object = match name.to_ascii_lowercase().as_str() {
            "application" => ExcelObject::Application,
            "worksheetfunction" => ExcelObject::Functions,
            "activesheet" => ExcelObject::Sheet(self.sheet),
            "thisworkbook" | "activeworkbook" => ExcelObject::Workbook,
            "worksheets" | "sheets" => ExcelObject::Sheets,
            "range" | "cells" => ExcelObject::Global(name.to_ascii_lowercase()),
            _ => return None,
        };
        self.object(object).ok()
    }
    fn scalar(&mut self, value: Value) -> gridcraft_vbarun::Result<Value> {
        if let Value::Object(Some(id)) = value {
            if let Some(ExcelObject::Range(area)) = self.objects.get(id as usize).cloned() {
                return self.values(area);
            }
            return Err(invalid());
        }
        Ok(value)
    }
    fn get_member(&mut self, id: Object, name: &str, args: &[Argument]) -> gridcraft_vbarun::Result<Value> {
        let object = self.objects.get(id as usize).cloned().ok_or_else(invalid)?;
        let name = name.to_ascii_lowercase();
        match (object, name.as_str()) {
            (ExcelObject::Global(global), "") => self.get_global(&global, self.sheet, args),
            (ExcelObject::Application, "volatile") => {
                if args.first().map(|a| a.value.number()).transpose()?.is_none_or(|v| v != 0.0) {
                    self.calc.note_volatile();
                }
                Ok(Value::Empty)
            }
            (ExcelObject::Application, "worksheetfunction") => self.object(ExcelObject::Functions),
            (ExcelObject::Application, "activesheet") => self.object(ExcelObject::Sheet(self.sheet)),
            (ExcelObject::Application, "range" | "cells") => self.get_global(&name, self.sheet, args),
            (ExcelObject::Sheet(sheet), "range" | "cells") => self.get_global(&name, sheet, args),
            (ExcelObject::Application | ExcelObject::Workbook, "sheets" | "worksheets") => {
                if args.is_empty() {
                    self.object(ExcelObject::Sheets)
                } else {
                    let sheets = self.object(ExcelObject::Sheets)?;
                    if let Value::Object(Some(id)) = sheets { self.get_member(id, "item", args) } else { Err(invalid()) }
                }
            }
            (ExcelObject::Application, "thisworkbook" | "activeworkbook") => self.object(ExcelObject::Workbook),
            (ExcelObject::Range(area), "rows" | "columns") if args.is_empty() => self.object(ExcelObject::Dimension(area, name == "rows")),
            (ExcelObject::Dimension(area, rows), "count") => Ok(Value::Double(if rows { area.range.height() } else { area.range.width() } as f64)),
            (ExcelObject::Sheets, "" | "item") => {
                let value = &args.first().ok_or_else(invalid)?.value;
                let sheet = if let Value::String(s) = value {
                    self.calc.workbook().sheet_index(s).ok_or_else(invalid)?
                } else {
                    usize::try_from(value.integer()?.checked_sub(1).ok_or_else(invalid)?).map_err(|_| invalid())?
                };
                if self.calc.workbook().sheet(sheet).is_none() {
                    return Err(invalid());
                }
                self.object(ExcelObject::Sheet(sheet))
            }
            (ExcelObject::Range(area), "value" | "value2" | "") if args.is_empty() => self.values(area),
            (ExcelObject::Range(area), "count") => Ok(Value::Double(area.range.count() as f64)),
            (ExcelObject::Range(area), "row") => Ok(Value::Double(f64::from(area.range.start.row) + 1.0)),
            (ExcelObject::Range(area), "column") => Ok(Value::Double(f64::from(area.range.start.col) + 1.0)),
            (ExcelObject::Range(area), "cells" | "item" | "") => {
                let row = args.first().map(|a| a.value.integer()).transpose()?.unwrap_or(1);
                let col = args.get(1).map(|a| a.value.integer()).transpose()?.unwrap_or(1);
                let cell =
                    area.range.start.offset(row.saturating_sub(1), col.saturating_sub(1)).filter(|c| area.range.contains(*c)).ok_or_else(invalid)?;
                self.object(ExcelObject::Range(Area { sheet: area.sheet, range: RangeRef::new(cell, cell) }))
            }
            (ExcelObject::Functions, _) => {
                let mut evaluated = Vec::with_capacity(args.len());
                for arg in args {
                    let value = self.scalar(arg.value.clone())?;
                    let value = match value {
                        Value::Array(a) => {
                            if a.bounds.len() > 2 {
                                return Err(invalid());
                            }
                            let dimension = |index| -> gridcraft_vbarun::Result<usize> {
                                a.bounds
                                    .get(index)
                                    .map(|(lo, hi)| usize::try_from(i64::from(*hi) - i64::from(*lo) + 1).map_err(|_| invalid()))
                                    .unwrap_or(Ok(1))
                            };
                            let (rows, cols) = (dimension(0)?, dimension(1)?);
                            let data = a.values.into_iter().map(to_cell).collect::<gridcraft_vbarun::Result<_>>()?;
                            CellValue::Array(std::sync::Arc::new(gridcraft_core::Array::new(rows, cols, data).ok_or_else(invalid)?))
                        }
                        v => to_cell(v)?,
                    };
                    evaluated.push(gridcraft_functions::Arg { value, from_ref: matches!(arg.value, Value::Object(_)) });
                }
                let spec = gridcraft_functions::lookup(&name.to_ascii_uppercase()).ok_or_else(invalid)?;
                from_cell(&gridcraft_functions::call(spec, &evaluated, &mut FunctionContext { calc: self.calc }))
            }
            _ => Err(denied()),
        }
    }
    fn set_member(&mut self, _obj: Object, _name: &str, _args: &[Argument], _value: Value) -> gridcraft_vbarun::Result<()> {
        Err(denied())
    }
    fn enumerate(&mut self, obj: Object) -> gridcraft_vbarun::Result<Vec<Value>> {
        let Some(ExcelObject::Range(area)) = self.objects.get(obj as usize).cloned() else { return Err(invalid()) };
        if area.range.count() > CELLS {
            return Err(resource_limit());
        }
        area.range.iter().map(|c| self.object(ExcelObject::Range(Area { sheet: area.sheet, range: RangeRef::new(c, c) }))).collect()
    }
}
impl ReadHost<'_> {
    fn get_global(&mut self, name: &str, sheet: usize, args: &[Argument]) -> gridcraft_vbarun::Result<Value> {
        if name == "range" {
            return self.range(sheet, args);
        }
        let row = args.first().ok_or_else(invalid)?.value.integer()?;
        let col = args.get(1).ok_or_else(invalid)?.value.integer()?;
        let row = u32::try_from(row.checked_sub(1).ok_or_else(invalid)?).map_err(|_| invalid())?;
        let col = u32::try_from(col.checked_sub(1).ok_or_else(invalid)?).map_err(|_| invalid())?;
        if row >= gridcraft_core::MAX_ROWS || col >= gridcraft_core::MAX_COLS {
            return Err(invalid());
        }
        let cell = CellRef::new(row, col);
        self.object(ExcelObject::Range(Area { sheet, range: RangeRef::new(cell, cell) }))
    }
}
struct FunctionContext<'a> {
    calc: &'a mut dyn CalcHost,
}
impl gridcraft_functions::Ctx for FunctionContext<'_> {
    fn date_system(&self) -> gridcraft_core::DateSystem {
        self.calc.workbook().date_system
    }
    fn now_serial(&self) -> f64 {
        self.calc.now_serial()
    }
    fn random(&mut self) -> f64 {
        self.calc.random()
    }
}
