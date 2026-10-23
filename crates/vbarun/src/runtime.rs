use crate::{
    Argument, Clock, Error, Host, Limits, Result, Value, intrinsic,
    parser::{Arg, Case, Decl, Expr, Procedure, Program, Statement, Stmt},
    value::{Array, default_value, mismatch, overflow},
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc, sync::atomic::Ordering};

#[derive(Clone)]
struct Slot {
    value: Value,
    ty: String,
    constant: bool,
    // Shared ByRef aliases must account for each slot once, without rescanning its arrays.
    memory: usize,
}
#[derive(Clone)]
struct Place {
    slot: Rc<RefCell<Slot>>,
    indices: Vec<i32>,
}
impl Place {
    fn with_value<R>(&self, f: impl FnOnce(&Value) -> R) -> Result<R> {
        let slot = self.slot.borrow();
        if self.indices.is_empty() {
            return Ok(f(&slot.value));
        }
        let Value::Array(a) = &slot.value else {
            return Err(mismatch());
        };
        a.values.get(a.offset(&self.indices)?).map(f).ok_or_else(|| Error::runtime(9, "Subscript out of range"))
    }
    fn read(&self) -> Result<Value> {
        self.with_value(Clone::clone)
    }
    fn object_value(&self) -> Result<Option<Value>> {
        self.with_value(|value| if let Value::Object(id) = value { Some(Value::Object(*id)) } else { None })
    }
    fn write(&self, v: Value) -> Result<()> {
        let mut slot = self.slot.borrow_mut();
        if slot.constant {
            return Err(Error::runtime(5, "Assignment to constant"));
        }
        if self.indices.is_empty() {
            let value = v.coerce(&slot.ty)?;
            slot.memory = value.memory();
            slot.value = value;
        } else {
            let Value::Array(a) = &mut slot.value else {
                return Err(mismatch());
            };
            let i = a.offset(&self.indices)?;
            let v = v.coerce(&a.element_type)?;
            let dest = a.values.get_mut(i).ok_or_else(|| Error::runtime(9, "Subscript out of range"))?;
            let old_memory = dest.memory();
            let new_memory = v.memory();
            *dest = v;
            slot.memory = slot.memory.saturating_sub(old_memory).saturating_add(new_memory);
        }
        Ok(())
    }
}
fn place(value: Value, ty: String, constant: bool) -> Place {
    let memory = value.memory();
    Place { slot: Rc::new(RefCell::new(Slot { value, ty, constant, memory })), indices: Vec::new() }
}
struct Frame {
    module: usize,
    procedure: String,
    vars: BTreeMap<String, Place>,
    code: Vec<Instruction>,
    labels: BTreeMap<String, usize>,
    pc: usize,
    handler: String,
    handling: bool,
    fault: Option<usize>,
    line: usize,
    with_base: usize,
    iteration_memory: usize,
}
#[derive(Clone)]
struct Instruction {
    line: usize,
    with_depth: usize,
    op: Op,
}
#[derive(Clone)]
enum Op {
    Simple(Statement),
    Jump(usize),
    Unless(Expr, usize),
    Label(String),
    ForInit(String, Expr, Expr, Expr, usize),
    ForNext(String, usize),
    EachInit(String, Expr, usize),
    EachNext(String, usize),
    With(Expr),
    EndWith,
}
struct LoopState {
    end: f64,
    step: f64,
}
struct EachState {
    values: Vec<Value>,
    index: usize,
}

/// A run owns its globals and static variables. No macro executes on construction or extraction.
pub struct Interpreter<'a, H: Host, C: Clock> {
    program: &'a Program,
    pub host: H,
    pub clock: C,
    pub limits: Limits,
    globals: Vec<BTreeMap<String, Place>>,
    statics: BTreeMap<String, Place>,
    frames: Vec<Frame>,
    steps: u64,
    quantum: usize,
    remaining: usize,
    expression_depth: usize,
    pub(crate) rng: u32,
    pub(crate) last_random: f64,
    pub(crate) err: Error,
    with: Vec<Value>,
    initialized: bool,
}
impl<'a, H: Host, C: Clock> Interpreter<'a, H, C> {
    pub fn new(program: &'a Program, host: H, clock: C, limits: Limits) -> Self {
        Self {
            program,
            host,
            clock,
            limits,
            globals: vec![BTreeMap::new(); program.modules.len()],
            statics: BTreeMap::new(),
            frames: Vec::new(),
            steps: 0,
            quantum: usize::MAX,
            remaining: usize::MAX,
            expression_depth: 0,
            rng: 0x50000,
            last_random: 0.0,
            err: Error::runtime(0, ""),
            with: Vec::new(),
            initialized: false,
        }
    }
    pub fn steps(&self) -> u64 {
        self.steps
    }
    /// Maximum instructions before returning control to the embedding frame loop.
    pub fn set_quantum(&mut self, instructions: usize) {
        self.quantum = instructions.clamp(1, 10_000);
        self.remaining = self.quantum;
    }
    pub fn run(&mut self, module: &str, procedure: &str, args: &[Argument]) -> Result<Value> {
        let mut future = Box::pin(self.run_async(module, procedure, args));
        let waker = std::task::Waker::noop();
        let mut context = std::task::Context::from_waker(waker);
        loop {
            if let std::task::Poll::Ready(result) = future.as_mut().poll(&mut context) {
                return result;
            }
        }
    }
    pub async fn run_async(&mut self, module: &str, procedure: &str, args: &[Argument]) -> Result<Value> {
        self.frames.clear();
        self.with.clear();
        self.expression_depth = 0;
        self.run_inner(module, procedure, args).await.map_err(|mut error| {
            if error.module.is_empty() {
                error.module = module.into();
                error.procedure = procedure.to_ascii_lowercase();
            }
            error
        })
    }
    async fn run_inner(&mut self, module: &str, procedure: &str, args: &[Argument]) -> Result<Value> {
        self.steps = 0;
        self.tick()?;
        let mut argument_memory = 0usize;
        for arg in args {
            self.validate(&arg.value)?;
            argument_memory = argument_memory.saturating_add(arg.value.memory());
            if argument_memory > self.limits.max_memory_bytes {
                return Err(Error::limit("argument memory"));
            }
        }
        if !self.initialized {
            for (i, m) in self.program.modules.iter().enumerate() {
                let frame = make_frame(i, "<module>", &[])?;
                self.frames.push(frame);
                for d in &m.declarations {
                    self.frame_mut()?.line = d.line;
                    self.declare(d, true).await.map_err(|e| self.annotate(e))?;
                }
                self.frames.pop();
            }
            self.initialized = true;
        }
        let i =
            self.program.modules.iter().position(|m| m.name.eq_ignore_ascii_case(module)).ok_or_else(|| Error::runtime(35, "Module not found"))?;
        let supplied = args.iter().map(|a| (a.clone(), None)).collect();
        Box::pin(self.invoke(i, &procedure.to_ascii_lowercase(), supplied)).await
    }
    pub fn global_value(&self, module: &str, name: &str) -> Option<Value> {
        let i = self.program.modules.iter().position(|m| m.name.eq_ignore_ascii_case(module))?;
        self.globals.get(i)?.get(&name.to_ascii_lowercase())?.read().ok()
    }
    fn frame(&self) -> Result<&Frame> {
        self.frames.last().ok_or_else(|| Error::runtime(5, "No active procedure"))
    }
    fn frame_mut(&mut self) -> Result<&mut Frame> {
        self.frames.last_mut().ok_or_else(|| Error::runtime(5, "No active procedure"))
    }
    pub(crate) fn tick(&mut self) -> Result<()> {
        self.steps = self.steps.saturating_add(1);
        if self.limits.cancellation.load(Ordering::Relaxed) {
            return Err(Error::limit("cancelled"));
        }
        if self.steps > self.limits.max_steps {
            return Err(Error::limit("instructions"));
        }
        if self.limits.deadline_ms.is_some_and(|d| self.clock.monotonic_ms() >= d) {
            return Err(Error::limit("deadline"));
        }
        Ok(())
    }
    pub(crate) fn validate(&self, v: &Value) -> Result<()> {
        self.validate_depth(v, 0)
    }
    fn validate_depth(&self, v: &Value, depth: usize) -> Result<()> {
        if depth > 32 {
            return Err(Error::limit("value nesting"));
        }
        match v {
            Value::String(s) if s.len() > self.limits.max_string_length => return Err(Error::limit("string length")),
            Value::Array(a) => {
                if a.values.len() > self.limits.max_array_elements {
                    return Err(Error::limit("array elements"));
                }
                for v in &a.values {
                    self.validate_depth(v, depth + 1)?;
                }
            }
            _ => {}
        }
        if v.memory() > self.limits.max_memory_bytes {
            return Err(Error::limit("memory"));
        }
        Ok(())
    }
    fn memory(&self) -> Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        let mut n = self.with.iter().fold(0usize, |n, v| n.saturating_add(v.memory()));
        for frame in &self.frames {
            n = n.saturating_add(frame.code.len().saturating_mul(std::mem::size_of::<Instruction>())).saturating_add(frame.iteration_memory);
        }
        for p in self.globals.iter().flat_map(|g| g.values()).chain(self.statics.values()).chain(self.frames.iter().flat_map(|f| f.vars.values())) {
            if seen.insert(Rc::as_ptr(&p.slot) as usize) {
                n = n.saturating_add(p.slot.borrow().memory);
            }
        }
        if n > self.limits.max_memory_bytes {
            return Err(Error::limit("memory"));
        }
        Ok(())
    }
    fn annotate(&self, mut e: Error) -> Error {
        if e.module.is_empty()
            && let Some(f) = self.frames.last()
        {
            e.module = self.program.modules.get(f.module).map_or(String::new(), |m| m.name.clone());
            e.procedure = f.procedure.clone();
            e.line = f.line;
        }
        e
    }
    async fn invoke(&mut self, module: usize, name: &str, args: Vec<(Argument, Option<Place>)>) -> Result<Value> {
        self.tick()?;
        if self.frames.len() >= self.limits.max_call_depth.min(128) {
            return Err(self.annotate(Error::limit("call depth")));
        }
        let module_name = self.program.modules.get(module).map(|m| m.name.as_str()).unwrap_or_default();
        self.host.enter_procedure(module_name, name)?;
        if let Some(error) = self.program.modules.get(module).and_then(|m| m.compile_error.as_ref()) {
            return Err(error.clone());
        }
        let proc = self
            .program
            .modules
            .get(module)
            .and_then(|m| m.procedures.get(name))
            .cloned()
            .ok_or_else(|| Error::runtime(35, format!("Procedure {name} not found")))?;
        let mut frame = make_frame(module, &proc.name, &proc.body)?;
        frame.vars.insert(proc.name.clone(), place(default_value(&proc.ty), proc.ty.clone(), false));
        frame.with_base = self.with.len();
        self.frames.push(frame);
        let with_len = self.with.len();
        let result = async {
            self.bind(&proc, args).await?;
            self.execute().await?;
            self.lookup(&proc.name)?.read()
        }
        .await;
        let result = result.map_err(|e| self.annotate(e));
        self.frames.pop();
        self.with.truncate(with_len);
        result
    }
    async fn bind(&mut self, proc: &Procedure, args: Vec<(Argument, Option<Place>)>) -> Result<()> {
        let mut used = vec![false; args.len()];
        for (index, param) in proc.params.iter().enumerate() {
            let supplied = args
                .iter()
                .enumerate()
                .find(|(i, (a, _))| {
                    !used.get(*i).copied().unwrap_or(true) && a.name.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(&param.decl.name))
                })
                .or_else(|| args.get(index).filter(|(a, _)| a.name.is_none()).map(|a| (index, a)));
            let p = if param.param_array {
                let mut values = Vec::new();
                for (i, (a, _)) in args.iter().enumerate() {
                    if !used.get(i).copied().unwrap_or(true) {
                        values.push(a.value.clone());
                        if let Some(u) = used.get_mut(i) {
                            *u = true;
                        }
                    }
                }
                self.validate(&Value::Array(Array {
                    bounds: vec![(0, values.len() as i32 - 1)],
                    values: values.clone(),
                    element_type: "variant".into(),
                    dynamic: true,
                }))?;
                place(
                    Value::Array(Array { bounds: vec![(0, values.len() as i32 - 1)], values, element_type: "variant".into(), dynamic: true }),
                    "variant".into(),
                    false,
                )
            } else if let Some((i, (a, p))) = supplied.filter(|(_, (a, _))| a.value != Value::Missing) {
                if let Some(u) = used.get_mut(i) {
                    *u = true;
                }
                if !param.by_val {
                    if let Some(p) = p { p.clone() } else { place(a.value.coerce(&param.decl.ty)?, param.decl.ty.clone(), false) }
                } else {
                    place(a.value.coerce(&param.decl.ty)?, param.decl.ty.clone(), false)
                }
            } else if param.optional {
                let v = if let Some(e) = &param.default {
                    Box::pin(self.eval(e)).await?
                } else if param.decl.ty == "variant" {
                    Value::Missing
                } else {
                    default_value(&param.decl.ty)
                };
                place(v, param.decl.ty.clone(), false)
            } else {
                return Err(Error::runtime(449, format!("Argument {} not optional", param.decl.name)));
            };
            self.frame_mut()?.vars.insert(param.decl.name.clone(), p);
        }
        if used.iter().enumerate().any(|(i, u)| !u && args.get(i).is_some_and(|(a, _)| a.value != Value::Missing)) {
            return Err(Error::runtime(450, "Wrong number or name of arguments"));
        }
        self.memory()
    }
    fn lookup(&self, name: &str) -> Result<Place> {
        let f = self.frame()?;
        f.vars
            .get(name)
            .or_else(|| self.globals.get(f.module).and_then(|g| g.get(name)))
            .or_else(|| {
                self.program.modules.iter().enumerate().find_map(|(index, module)| {
                    if module.declarations.iter().any(|d| d.name == name && d.public) { self.globals.get(index)?.get(name) } else { None }
                })
            })
            .cloned()
            .ok_or_else(|| Error::runtime(35, format!("Variable {name} not defined")))
    }
    fn variable(&mut self, name: &str) -> Result<Place> {
        if let Ok(p) = self.lookup(name) {
            return Ok(p);
        }
        if self.program.modules.get(self.frame()?.module).is_some_and(|m| m.option_explicit) {
            return Err(Error::runtime(35, format!("Variable {name} not defined (Option Explicit)")));
        }
        let p = place(default_value(&crate::parser::suffix_type(name)), crate::parser::suffix_type(name), false);
        self.frame_mut()?.vars.insert(name.into(), p.clone());
        Ok(p)
    }
    fn is_location(&self, e: &Expr) -> bool {
        match e {
            Expr::Name(n) => self.lookup(n).is_ok(),
            Expr::Invoke(base, _) => self.is_location(base),
            _ => false,
        }
    }
    async fn location(&mut self, e: &Expr) -> Result<Place> {
        match e {
            Expr::Name(n) => self.variable(n),
            Expr::Invoke(base, args) => {
                let mut p = Box::pin(self.location(base)).await?;
                for a in args {
                    p.indices.push(i32::try_from(Box::pin(self.eval_scalar(&a.expr)).await?.integer()?).map_err(|_| overflow())?);
                }
                p.read()?;
                Ok(p)
            }
            _ => Err(mismatch()),
        }
    }
    async fn bounds(&mut self, d: &Decl) -> Result<Vec<(i32, i32)>> {
        let mut bounds = Vec::new();
        for (lo, hi) in d.bounds.as_deref().unwrap_or_default() {
            let lo = if let Some(e) = lo {
                i32::try_from(Box::pin(self.eval_scalar(e)).await?.integer()?).map_err(|_| overflow())?
            } else {
                self.program.modules.get(self.frame()?.module).map_or(0, |m| m.option_base)
            };
            let hi = i32::try_from(Box::pin(self.eval_scalar(hi)).await?.integer()?).map_err(|_| overflow())?;
            bounds.push((lo, hi));
        }
        Ok(bounds)
    }
    async fn declare(&mut self, d: &Decl, global: bool) -> Result<()> {
        let key = format!("{}:{}:{}", self.frame()?.module, self.frame()?.procedure, d.name);
        let p = if d.persistent && self.statics.contains_key(&key) {
            self.statics.get(&key).cloned().ok_or_else(mismatch)?
        } else {
            let value = if d.bounds.is_some() {
                Value::Array(Array::new(self.bounds(d).await?, d.ty.clone(), d.bounds.as_ref().is_some_and(Vec::is_empty), self.array_cap())?)
            } else if let Some(e) = &d.initial {
                Box::pin(self.eval_scalar(e)).await?.coerce(&d.ty)?
            } else {
                default_value(&d.ty)
            };
            self.validate(&value)?;
            let p = place(value, if d.bounds.is_some() { "variant".into() } else { d.ty.clone() }, d.constant);
            if d.persistent {
                self.statics.insert(key, p.clone());
            }
            p
        };
        if global {
            let i = self.frame()?.module;
            if let Some(g) = self.globals.get_mut(i) {
                g.insert(d.name.clone(), p);
            }
        } else {
            self.frame_mut()?.vars.insert(d.name.clone(), p);
        }
        self.memory()
    }
    pub(crate) fn option_base(&self) -> i32 {
        self.frames.last().and_then(|f| self.program.modules.get(f.module)).map_or(0, |m| m.option_base)
    }
    pub(crate) fn compare_text(&self) -> bool {
        self.frames.last().and_then(|f| self.program.modules.get(f.module)).is_some_and(|m| m.compare_text)
    }
    pub(crate) fn array_cap(&self) -> usize {
        self.limits.max_array_elements.min(self.limits.max_memory_bytes / 32)
    }
    async fn assign(&mut self, target: &Expr, value: Value) -> Result<()> {
        self.validate(&value)?;
        match target {
            Expr::Evaluate(_, _) => {
                let id = object_id(Box::pin(self.eval(target)).await?)?;
                self.host.set_member_async(id, "", &[], value).await?;
            }
            Expr::Member(base, name) => {
                if matches!(base.as_ref(), Expr::Name(object) if object == "err") {
                    match name.as_str() {
                        "number" => self.err.number = i32::try_from(value.integer()?).map_err(|_| overflow())?,
                        "description" => self.err.description = value.text()?,
                        "source" => self.err.source = value.text()?,
                        _ => return Err(Error::runtime(438, "Unknown Err member")),
                    }
                    return Ok(());
                }
                if let Expr::Name(module_name) = base.as_ref()
                    && let Some(module) = self.program.modules.iter().position(|m| m.name.eq_ignore_ascii_case(module_name))
                    && self.program.modules.get(module).is_some_and(|m| m.declarations.iter().any(|d| d.name == *name))
                {
                    self.module_variable(module, name)?.write(value)?;
                    return self.memory();
                }
                let object = Box::pin(self.eval(base)).await?;
                let id = object_id(object)?;
                self.host.set_member_async(id, name, &[], value).await?;
            }
            Expr::Invoke(base, args) if matches!(base.as_ref(), Expr::Member(_, _)) => {
                if let Expr::Member(obj, name) = base.as_ref() {
                    let id = object_id(Box::pin(self.eval(obj)).await?)?;
                    let args = self.arguments(args).await?;
                    self.host.set_member_async(id, name, &args, value).await?;
                }
            }
            Expr::Invoke(base, args) => {
                if let Expr::Name(name) = base.as_ref() {
                    let kind = if matches!(value, Value::Object(_)) { "set" } else { "let" };
                    let setter = format!("{name}.{kind}");
                    let module = self.frame()?.module;
                    if self.program.modules.get(module).is_some_and(|m| m.procedures.contains_key(&setter)) {
                        let mut supplied: Vec<_> = self.arguments(args).await?.into_iter().map(|arg| (arg, None)).collect();
                        supplied.push((Argument::positional(value), None));
                        Box::pin(self.invoke(module, &setter, supplied)).await?;
                        return self.memory();
                    }
                    let object = match self.lookup(name) {
                        Ok(p) => p.object_value()?,
                        Err(_) => self.host.global(name),
                    };
                    if let Some(Value::Object(id)) = object {
                        let id = object_id(Value::Object(id))?;
                        let args = self.arguments(args).await?;
                        self.host.set_member_async(id, "", &args, value).await?;
                        return self.memory();
                    }
                }
                Box::pin(self.location(target)).await?.write(value)?;
            }
            Expr::Name(n)
                if self.lookup(n).is_err()
                    && self
                        .program
                        .modules
                        .get(self.frame()?.module)
                        .is_some_and(|m| m.procedures.contains_key(&format!("{n}.let")) || m.procedures.contains_key(&format!("{n}.set"))) =>
            {
                let kind = if matches!(value, Value::Object(_)) { "set" } else { "let" };
                Box::pin(self.invoke(self.frame()?.module, &format!("{n}.{kind}"), vec![(Argument::positional(value), None)])).await?;
            }
            _ => Box::pin(self.location(target)).await?.write(value)?,
        }
        self.memory()
    }
    async fn indices(&mut self, args: &[Arg]) -> Result<Vec<i32>> {
        let mut values = Vec::new();
        for a in args {
            values.push(i32::try_from(Box::pin(self.eval_scalar(&a.expr)).await?.integer()?).map_err(|_| overflow())?);
        }
        Ok(values)
    }
    async fn arguments(&mut self, args: &[Arg]) -> Result<Vec<Argument>> {
        let mut arguments = Vec::new();
        let mut memory = 0usize;
        for arg in args {
            let value = Box::pin(self.eval(&arg.expr)).await?;
            memory = memory.saturating_add(value.memory());
            if memory > self.limits.max_memory_bytes {
                return Err(Error::limit("argument memory"));
            }
            arguments.push(Argument { name: arg.name.clone(), value });
        }
        Ok(arguments)
    }
    async fn supplied_arguments(&mut self, args: &[Arg]) -> Result<Vec<(Argument, Option<Place>)>> {
        let mut supplied = Vec::new();
        let mut argument_memory = 0usize;
        for a in args {
            let implicit_variable = if let Expr::Name(n) = &a.expr {
                !intrinsic::exists(n)
                    && constant(n).is_none()
                    && !self.program.modules.iter().any(|m| m.procedures.contains_key(n))
                    && self.host.global(n).is_none()
            } else {
                false
            };
            let p = if self.is_location(&a.expr) || implicit_variable { Some(Box::pin(self.location(&a.expr)).await?) } else { None };
            let value = if let Some(p) = &p { p.read()? } else { Box::pin(self.eval(&a.expr)).await? };
            argument_memory = argument_memory.saturating_add(value.memory());
            if argument_memory > self.limits.max_memory_bytes {
                return Err(Error::limit("argument memory"));
            }
            supplied.push((Argument { name: a.name.clone(), value }, p));
        }
        Ok(supplied)
    }
    fn module_variable(&self, module: usize, name: &str) -> Result<Place> {
        let metadata = self.program.modules.get(module).ok_or_else(mismatch)?;
        let visible = module == self.frame()?.module || metadata.declarations.iter().any(|d| d.name == name && d.public);
        if !visible {
            return Err(Error::runtime(35, format!("Variable {name} is private")));
        }
        self.globals.get(module).and_then(|g| g.get(name)).cloned().ok_or_else(|| Error::runtime(35, format!("Variable {name} not found")))
    }
    async fn call(&mut self, target: &Expr, args: &[Arg], method: bool) -> Result<Value> {
        if let Expr::Name(name) = target {
            if let Ok(p) = self.lookup(name) {
                if p.with_value(|value| matches!(value, Value::Array(_)))? {
                    let indices = self.indices(args).await?;
                    return p.with_value(|value| {
                        let Value::Array(a) = value else { return Err(mismatch()) };
                        a.values.get(a.offset(&indices)?).cloned().ok_or_else(|| Error::runtime(9, "Subscript out of range"))
                    })?;
                }
                if let Some(Value::Object(id)) = p.object_value()? {
                    let id = object_id(Value::Object(id))?;
                    let args = self.arguments(args).await?;
                    return self.host.get_member_async(id, "", &args).await;
                }
            }
            let module = self.frame()?.module;
            let proc_module = if self.program.modules.get(module).is_some_and(|m| m.procedures.contains_key(name)) {
                Some(module)
            } else {
                self.program.modules.iter().position(|m| m.procedures.get(name).is_some_and(|p| p.public))
            };
            if let Some(i) = proc_module {
                let supplied = self.supplied_arguments(args).await?;
                return Box::pin(self.invoke(i, name, supplied)).await;
            }
            let args = self.arguments(args).await?;
            if let Some(value) = self.host.global(name) {
                self.validate(&value)?;
                return self.host.get_member_async(object_id(value)?, "", &args).await;
            }
            match name.trim_end_matches('$') {
                "msgbox" => return self.host.msg_box_async(&args).await,
                "inputbox" => return self.host.input_box_async(&args).await,
                "createobject" | "getobject" => return self.host.create_object(name == "getobject", &args),
                _ => {}
            }
            let args = args
                .into_iter()
                .map(|mut a| {
                    a.value = self.host.scalar(a.value)?;
                    Ok(a)
                })
                .collect::<Result<Vec<_>>>()?;
            return intrinsic::call(self, name.trim_end_matches('$'), &args);
        }
        if let Expr::Member(obj, name) = target {
            if matches!(obj.as_ref(), Expr::Name(n) if n == "err") {
                let args = self.arguments(args).await?;
                return match name.as_str() {
                    "clear" => {
                        self.err = Error::runtime(0, "");
                        Ok(Value::Empty)
                    }
                    "raise" => {
                        let number = args.first().map(|a| a.value.integer()).transpose()?.unwrap_or(5) as i32;
                        let source = args.get(1).map(|a| a.value.text()).transpose()?.unwrap_or_default();
                        let description = args.get(2).map(|a| a.value.text()).transpose()?.unwrap_or_else(|| "Application-defined error".into());
                        let mut e = Error::runtime(number, description);
                        e.source = source;
                        Err(e)
                    }
                    "number" => Ok(Value::Long(self.err.number)),
                    "description" => Ok(Value::String(self.err.description.clone())),
                    "source" => Ok(Value::String(self.err.source.clone())),
                    _ => Err(Error::runtime(438, "Unknown Err member")),
                };
            }
            if let Expr::Name(module_name) = obj.as_ref()
                && let Some(i) = self.program.modules.iter().position(|m| m.name.eq_ignore_ascii_case(module_name))
            {
                if self.program.modules.get(i).is_some_and(|m| m.declarations.iter().any(|d| d.name == *name)) {
                    let value = self.module_variable(i, name)?.read()?;
                    if args.is_empty() {
                        return Ok(value);
                    }
                    if let Value::Array(array) = value {
                        let indices = self.indices(args).await?;
                        return array.values.get(array.offset(&indices)?).cloned().ok_or_else(|| Error::runtime(9, "Subscript out of range"));
                    }
                    let args = self.arguments(args).await?;
                    return self.host.get_member_async(object_id(value)?, "", &args).await;
                }
                if !self.program.modules.get(i).is_some_and(|m| m.procedures.contains_key(name)) {
                    let object = self.lookup(module_name).ok().map(|p| p.read()).transpose()?.or_else(|| self.host.global(module_name));
                    if let Some(value) = object {
                        let args = self.arguments(args).await?;
                        let id = object_id(value)?;
                        return if method {
                            self.host.call_method_async(id, name, &args).await
                        } else {
                            self.host.get_member_async(id, name, &args).await
                        };
                    }
                    if let Some(error) = self.program.modules.get(i).and_then(|m| m.compile_error.as_ref()) {
                        return Err(error.clone());
                    }
                }
                let visible = i == self.frame()?.module || self.program.modules.get(i).and_then(|m| m.procedures.get(name)).is_some_and(|p| p.public);
                if !visible {
                    return Err(Error::runtime(35, format!("Procedure {name} is private")));
                }
                let args = self.supplied_arguments(args).await?;
                return Box::pin(self.invoke(i, name, args)).await;
            }
            let id = object_id(Box::pin(self.eval(obj)).await?)?;
            let args = self.arguments(args).await?;
            return if method { self.host.call_method_async(id, name, &args).await } else { self.host.get_member_async(id, name, &args).await };
        }
        let value = Box::pin(self.eval(target)).await?;
        if let Value::Array(array) = value {
            let indices = self.indices(args).await?;
            return array.values.get(array.offset(&indices)?).cloned().ok_or_else(|| Error::runtime(9, "Subscript out of range"));
        }
        let id = object_id(value)?;
        let args = self.arguments(args).await?;
        self.host.get_member_async(id, "", &args).await
    }
    async fn eval_scalar(&mut self, expression: &Expr) -> Result<Value> {
        let value = Box::pin(self.eval(expression)).await?;
        self.host.scalar(value)
    }
    async fn eval(&mut self, e: &Expr) -> Result<Value> {
        self.tick()?;
        if self.expression_depth >= 128 {
            return Err(Error::limit("expression depth"));
        }
        self.expression_depth += 1;
        let result = self.eval_inner(e).await;
        self.expression_depth -= 1;
        result
    }
    async fn eval_inner(&mut self, e: &Expr) -> Result<Value> {
        let value = match e {
            Expr::Literal(v) => v.clone(),
            Expr::Evaluate(base, text) => {
                if text.len() > self.limits.max_string_length {
                    return Err(Error::limit("string length"));
                }
                let expression = Value::String(text.clone());
                self.validate(&expression)?;
                let id = if let Some(base) = base {
                    Box::pin(self.eval(base)).await?
                } else {
                    self.host.global("application").ok_or_else(|| Error::unsupported("Evaluate"))?
                };
                self.host.get_member_async(object_id(id)?, "evaluate", &[Argument::positional(expression)]).await?
            }
            Expr::With => {
                if self.with.len() <= self.frame()?.with_base {
                    return Err(Error::runtime(91, "With block not active"));
                }
                self.with.last().cloned().ok_or_else(|| Error::runtime(91, "With block not active"))?
            }
            Expr::Name(n) => {
                if let Ok(p) = self.lookup(n) {
                    p.read()?
                } else if let Some(v) = constant(n) {
                    v
                } else if let Some(v) = self.host.global(n) {
                    v
                } else if intrinsic::exists(n) || self.program.modules.iter().any(|m| m.procedures.contains_key(n)) {
                    self.call(e, &[], false).await?
                } else {
                    self.variable(n)?.read()?
                }
            }
            Expr::Member(_, _) => self.call(e, &[], false).await?,
            Expr::Invoke(target, args) => self.call(target, args, false).await?,
            Expr::Unary(op, e) => {
                let v = Box::pin(self.eval(e)).await?;
                let v = self.host.scalar(v)?;
                if v == Value::Null {
                    Value::Null
                } else {
                    match op.as_str() {
                        "-" => Value::Double(-v.number()?),
                        "+" => Value::Double(v.number()?),
                        "not" => {
                            if let Value::Boolean(b) = v {
                                Value::Boolean(!b)
                            } else {
                                Value::LongLong(!v.integer()?)
                            }
                        }
                        _ => return Err(mismatch()),
                    }
                }
            }
            Expr::Binary(op, a, b) => {
                let left = Box::pin(self.eval(a)).await?;
                let right = Box::pin(self.eval(b)).await?;
                let left = self.host.scalar(left)?;
                let right = self.host.scalar(right)?;
                let variants = (self.variant_expression(a), self.variant_expression(b));
                self.binary(op, left, right, variants)?
            }
        };
        self.validate(&value)?;
        Ok(value)
    }
    fn variant_expression(&self, expression: &Expr) -> bool {
        match expression {
            Expr::Literal(_) => false,
            Expr::Name(name) => self.lookup(name).is_ok_and(|p| p.slot.borrow().ty == "variant"),
            Expr::Unary(_, inner) => self.variant_expression(inner),
            Expr::Binary(_, left, right) => self.variant_expression(left) || self.variant_expression(right),
            Expr::Invoke(target, _) => match target.as_ref() {
                Expr::Name(name) => {
                    self.program.modules.iter().find_map(|m| m.procedures.get(name)).map_or_else(|| !intrinsic::exists(name), |p| p.ty == "variant")
                }
                _ => true,
            },
            _ => true,
        }
    }
    fn binary(&mut self, op: &str, a: Value, b: Value, variants: (bool, bool)) -> Result<Value> {
        let promote = variants.0 || variants.1;
        if op == "is" {
            return match (&a, &b) {
                (Value::Object(a), Value::Object(b)) => Ok(Value::Boolean(a == b)),
                _ => Err(mismatch()),
            };
        }
        if op == "&" {
            return Ok(Value::String(format!(
                "{}{}",
                if a == Value::Null { String::new() } else { a.text()? },
                if b == Value::Null { String::new() } else { b.text()? }
            )));
        }
        if a == Value::Null || b == Value::Null {
            let other = if a == Value::Null { &b } else { &a };
            if *other != Value::Null {
                match op {
                    "and" if other.integer()? == 0 => return Ok(other.clone()),
                    "or" => return Ok(other.clone()),
                    "imp" if a == Value::Null => return Ok(b.clone()),
                    _ => {}
                }
            }
            return Ok(Value::Null);
        }
        if ["=", "<>", "<", ">", "<=", ">=", "like"].contains(&op) {
            let text = self.program.modules.get(self.frame()?.module).is_some_and(|m| m.compare_text);
            if op == "like" {
                return Ok(Value::Boolean(intrinsic::like(&a.text()?, &b.text()?, text, || self.tick())?));
            }
            let strings = matches!((&a, &b), (Value::String(_), Value::String(_) | Value::Empty) | (Value::Empty, Value::String(_)))
                || (matches!(a, Value::String(_)) && !variants.0 && variants.1)
                || (matches!(b, Value::String(_)) && !variants.1 && variants.0);
            let order = if strings {
                let (a, b) = (a.text()?, b.text()?);
                if text { a.to_lowercase().cmp(&b.to_lowercase()) } else { a.cmp(&b) }
            } else if variants.0 && variants.1 && matches!(a, Value::String(_)) && b.number().is_ok() {
                std::cmp::Ordering::Greater
            } else if variants.0 && variants.1 && matches!(b, Value::String(_)) && a.number().is_ok() {
                std::cmp::Ordering::Less
            } else {
                a.number()?.partial_cmp(&b.number()?).ok_or_else(mismatch)?
            };
            return Ok(Value::Boolean(match op {
                "=" => order.is_eq(),
                "<>" => !order.is_eq(),
                "<" => order.is_lt(),
                ">" => order.is_gt(),
                "<=" => !order.is_gt(),
                _ => !order.is_lt(),
            }));
        }
        if ["and", "or", "xor", "eqv", "imp"].contains(&op) {
            let boolean = matches!((&a, &b), (Value::Boolean(_), Value::Boolean(_)));
            let ty = if matches!((&a, &b), (Value::LongLong(_), _) | (_, Value::LongLong(_))) {
                "longlong"
            } else if matches!(
                (&a, &b),
                (Value::Long(_) | Value::Single(_) | Value::Double(_) | Value::Currency(_) | Value::Date(_) | Value::String(_), _)
                    | (_, Value::Long(_) | Value::Single(_) | Value::Double(_) | Value::Currency(_) | Value::Date(_) | Value::String(_))
            ) {
                "long"
            } else if matches!((&a, &b), (Value::Byte(_), Value::Byte(_))) {
                "byte"
            } else {
                "integer"
            };
            let (x, y) = (a.coerce(ty)?.integer()?, b.coerce(ty)?.integer()?);
            let n = match op {
                "and" => x & y,
                "or" => x | y,
                "xor" => x ^ y,
                "eqv" => !(x ^ y),
                _ => !x | y,
            };
            return if boolean { Ok(Value::Boolean(n != 0)) } else { Value::LongLong(n).coerce(ty) };
        }
        if op == "+" && matches!((&a, &b), (Value::String(_), Value::String(_) | Value::Empty) | (Value::Empty, Value::String(_))) {
            return Ok(Value::String(format!("{}{}", a.text()?, b.text()?)));
        }
        if let (Value::Currency(x), Value::Currency(y)) = (&a, &b) {
            if op == "+" {
                return Ok(Value::Currency(x.checked_add(*y).ok_or_else(overflow)?));
            }
            if op == "-" {
                return Ok(Value::Currency(x.checked_sub(*y).ok_or_else(overflow)?));
            }
        }
        let (x, y) = (a.number()?, b.number()?);
        let n = match op {
            "+" => x + y,
            "-" => x - y,
            "*" => x * y,
            "^" => x.powf(y),
            "/" => {
                if y == 0.0 {
                    return Err(Error::runtime(11, "Division by zero"));
                }
                x / y
            }
            "\\" | "mod" => {
                let wide = matches!((&a, &b), (Value::LongLong(_), _) | (_, Value::LongLong(_)));
                let operand_type = if wide { "longlong" } else { "long" };
                let (x, y) = (a.coerce(operand_type)?.integer()?, b.coerce(operand_type)?.integer()?);
                if y == 0 {
                    return Err(Error::runtime(11, "Division by zero"));
                }
                let n = if op == "mod" { x.checked_rem(y) } else { x.checked_div(y) }.ok_or_else(overflow)?;
                let ty = if matches!((&a, &b), (Value::LongLong(_), _) | (_, Value::LongLong(_))) {
                    "longlong"
                } else if matches!(
                    (&a, &b),
                    (Value::Integer(_) | Value::Byte(_) | Value::Boolean(_), Value::Integer(_) | Value::Byte(_) | Value::Boolean(_))
                ) {
                    "integer"
                } else {
                    "long"
                };
                return Value::LongLong(n).coerce(ty);
            }
            _ => return Err(mismatch()),
        };
        if !n.is_finite() {
            return Err(overflow());
        }
        let ty = arithmetic_type(op, &a, &b);
        if ty == "longlong" && ["+", "-", "*"].contains(&op) {
            let (x, y) = (a.integer()?, b.integer()?);
            return Ok(Value::LongLong(
                match op {
                    "+" => x.checked_add(y),
                    "-" => x.checked_sub(y),
                    _ => x.checked_mul(y),
                }
                .ok_or_else(overflow)?,
            ));
        }
        let result = Value::Double(n).coerce(ty);
        match result {
            Err(error) if error.number == 6 && promote && ["integer", "long", "single"].contains(&ty) => {
                if (i32::MIN as f64..=i32::MAX as f64).contains(&n) && n.fract() == 0.0 { Ok(Value::Long(n as i32)) } else { Ok(Value::Double(n)) }
            }
            result => result,
        }
    }
    async fn execute(&mut self) -> Result<()> {
        let mut loops: BTreeMap<String, LoopState> = BTreeMap::new();
        let mut each: BTreeMap<String, EachState> = BTreeMap::new();
        loop {
            self.tick().map_err(|e| self.annotate(e))?;
            if self.remaining == 0 {
                YieldOnce(false).await;
                self.remaining = self.quantum;
                self.tick().map_err(|e| self.annotate(e))?;
            }
            self.remaining = self.remaining.saturating_sub(1);
            let pc = self.frame()?.pc;
            let Some(instruction) = self.frame()?.code.get(pc).cloned() else {
                return Ok(());
            };
            self.frame_mut()?.line = instruction.line;
            let with_depth = self.frame()?.with_base.saturating_add(instruction.with_depth);
            if self.with.len() > with_depth {
                self.with.truncate(with_depth);
            }
            self.frame_mut()?.pc = pc.saturating_add(1);
            let result = self.instruction(instruction.op, &mut loops, &mut each).await;
            if let Err(e) = result {
                let e = self.annotate(e);
                let handler = self.frame()?.handler.clone();
                if e.fatal || handler.is_empty() || self.frame()?.handling {
                    return Err(e);
                }
                self.err = e;
                if self.err.source.is_empty() {
                    self.err.source = format!("{}.{}", self.err.module, self.err.procedure);
                }
                self.frame_mut()?.fault = Some(pc);
                if handler != "next" {
                    self.frame_mut()?.handling = true;
                    self.goto(&handler)?;
                }
            }
        }
    }
    fn goto(&mut self, label: &str) -> Result<()> {
        let dest = self.frame()?.labels.get(label).copied().ok_or_else(|| Error::runtime(35, format!("Label {label} not defined")))?;
        self.frame_mut()?.pc = dest;
        Ok(())
    }
    async fn instruction(&mut self, op: Op, loops: &mut BTreeMap<String, LoopState>, each: &mut BTreeMap<String, EachState>) -> Result<()> {
        match op {
            Op::Jump(dest) => self.frame_mut()?.pc = dest,
            Op::Unless(e, dest) => {
                if !Box::pin(self.eval_scalar(&e)).await?.truth()? {
                    self.frame_mut()?.pc = dest;
                }
            }
            Op::Label(_) => {}
            Op::ForInit(name, start, end, step, dest) => {
                let start = Box::pin(self.eval_scalar(&start)).await?;
                let end = Box::pin(self.eval_scalar(&end)).await?.number()?;
                let step = Box::pin(self.eval_scalar(&step)).await?.number()?;
                let n = start.number()?;
                self.assign(&Expr::Name(name.clone()), start).await?;
                loops.insert(name, LoopState { end, step });
                if (step >= 0.0 && n > end) || (step < 0.0 && n < end) {
                    self.frame_mut()?.pc = dest;
                }
            }
            Op::ForNext(name, dest) => {
                let state = loops.get(&name).ok_or_else(|| Error::runtime(92, "For loop not initialized"))?;
                let n = self.variable(&name)?.read()?.number()? + state.step;
                let again = if state.step >= 0.0 { n <= state.end } else { n >= state.end };
                self.assign(&Expr::Name(name), Value::Double(n)).await?;
                if again {
                    self.frame_mut()?.pc = dest;
                }
            }
            Op::EachInit(name, expr, dest) => {
                let values = match Box::pin(self.eval(&expr)).await? {
                    Value::Array(a) => a.values,
                    Value::Object(Some(id)) => self.host.enumerate(id)?,
                    _ => return Err(mismatch()),
                };
                if values.len() > self.array_cap() {
                    return Err(Error::limit("enumeration elements"));
                }
                for value in &values {
                    self.validate(value)?;
                }
                if let Some(v) = values.first() {
                    self.assign(&Expr::Name(name.clone()), v.clone()).await?;
                } else {
                    self.frame_mut()?.pc = dest;
                }
                each.insert(name, EachState { values, index: 0 });
            }
            Op::EachNext(name, dest) => {
                let state = each.get_mut(&name).ok_or_else(mismatch)?;
                state.index = state.index.saturating_add(1);
                if let Some(v) = state.values.get(state.index) {
                    self.assign(&Expr::Name(name), v.clone()).await?;
                    self.frame_mut()?.pc = dest;
                }
            }
            Op::With(e) => {
                let v = Box::pin(self.eval(&e)).await?;
                self.with.push(v);
            }
            Op::EndWith => {
                self.with.pop();
            }
            Op::Simple(s) => match s {
                Statement::Decl(ds) => {
                    for d in &ds {
                        self.declare(d, false).await?;
                    }
                }
                Statement::Assign(dest, value) => {
                    let v = Box::pin(self.eval(&value)).await?;
                    let v = self.host.scalar(v)?;
                    let object = if let Expr::Name(name) = &dest {
                        match self.lookup(name) {
                            Ok(p) => p.object_value()?,
                            Err(_) => self.host.global(name),
                        }
                    } else {
                        None
                    };
                    if let Some(Value::Object(Some(id))) = object {
                        self.host.set_member_async(id, "", &[], v).await?;
                    } else {
                        self.assign(&dest, v).await?;
                    }
                }
                Statement::SetAssign(dest, value) => {
                    let v = Box::pin(self.eval(&value)).await?;
                    self.assign(&dest, v).await?;
                }
                Statement::Call(e) => {
                    if let Expr::Invoke(target, args) = e {
                        self.call(&target, &args, true).await?;
                    } else {
                        self.call(&e, &[], true).await?;
                    }
                }
                Statement::Goto(label) => self.goto(&label)?,
                Statement::OnError(handler) => {
                    self.frame_mut()?.handler = if handler == "0" { String::new() } else { handler };
                    self.frame_mut()?.handling = false;
                }
                Statement::Resume(label) => {
                    let pc = self.frame()?.fault.ok_or_else(|| Error::runtime(20, "Resume without error"))?;
                    self.frame_mut()?.handling = false;
                    self.frame_mut()?.fault = None;
                    self.err = Error::runtime(0, "");
                    if label.is_empty() {
                        self.frame_mut()?.pc = pc;
                    } else if label == "next" {
                        self.frame_mut()?.pc = pc.saturating_add(1);
                    } else {
                        self.goto(&label)?;
                    }
                }
                Statement::Redim(preserve, ds) => {
                    for d in &ds {
                        let p = self.variable(&d.name)?;
                        let old = p.read()?;
                        let (ty, dynamic) = if let Value::Array(a) = &old { (a.element_type.clone(), a.dynamic) } else { (d.ty.clone(), true) };
                        if !dynamic {
                            return Err(Error::runtime(10, "Array is fixed"));
                        }
                        let mut a = Array::new(self.bounds(d).await?, ty, true, self.array_cap())?;
                        if preserve
                            && let Value::Array(old) = old
                            && !old.bounds.is_empty()
                        {
                            if old.bounds.len() != a.bounds.len()
                                || old.bounds.iter().zip(&a.bounds).enumerate().any(|(i, (x, y))| x.0 != y.0 || (i + 1 < a.bounds.len() && x != y))
                            {
                                return Err(Error::runtime(9, "ReDim Preserve can change only the last upper bound"));
                            }
                            for (index, from) in old.values.into_iter().enumerate() {
                                let mut remainder = index;
                                let mut coordinates = Vec::with_capacity(old.bounds.len());
                                for (lo, hi) in old.bounds.iter().rev() {
                                    let width = (i64::from(*hi) - i64::from(*lo) + 1) as usize;
                                    if width == 0 {
                                        break;
                                    }
                                    coordinates.push((i64::from(*lo) + (remainder % width) as i64) as i32);
                                    remainder /= width;
                                }
                                coordinates.reverse();
                                if let Ok(offset) = a.offset(&coordinates)
                                    && let Some(to) = a.values.get_mut(offset)
                                {
                                    *to = from;
                                }
                            }
                        }
                        self.validate(&Value::Array(a.clone()))?;
                        p.write(Value::Array(a))?;
                    }
                }
                Statement::Erase(es) => {
                    for e in &es {
                        let p = Box::pin(self.location(e)).await?;
                        let Value::Array(a) = p.read()? else {
                            return Err(mismatch());
                        };
                        let b = if a.dynamic { Vec::new() } else { a.bounds };
                        p.write(Value::Array(Array::new(b, a.element_type, a.dynamic, self.array_cap())?))?;
                    }
                }
                Statement::Mid(dest, start, len, replacement) => {
                    let original = Box::pin(self.eval_scalar(&dest)).await?.text()?;
                    let start = Box::pin(self.eval_scalar(&start)).await?.integer()?;
                    if start < 1 {
                        return Err(Error::runtime(5, "Invalid Mid start"));
                    }
                    let replacement = Box::pin(self.eval_scalar(&replacement)).await?.text()?;
                    let len =
                        if let Some(e) = len { Box::pin(self.eval_scalar(&e)).await?.integer()? } else { replacement.encode_utf16().count() as i64 };
                    if len < 0 {
                        return Err(Error::runtime(5, "Invalid Mid length"));
                    }
                    let mut units: Vec<u16> = original.encode_utf16().collect();
                    let start = (start - 1) as usize;
                    for (i, u) in replacement.encode_utf16().take(len as usize).enumerate() {
                        if let Some(to) = units.get_mut(start.saturating_add(i)) {
                            *to = u;
                        }
                    }
                    self.assign(&dest, Value::String(String::from_utf16_lossy(&units))).await?;
                }
                Statement::Print(es) => {
                    let mut out = String::new();
                    for e in &es {
                        let text = Box::pin(self.eval_scalar(e)).await?.text()?;
                        let length = out.len().saturating_add(text.len()).saturating_add(usize::from(!out.is_empty()));
                        if length > self.limits.max_string_length || length > self.limits.max_memory_bytes {
                            return Err(Error::limit("log length"));
                        }
                        if !out.is_empty() {
                            out.push(' ');
                        }
                        out.push_str(&text);
                    }
                    self.host.log(&out);
                }
                Statement::Unsupported(feature) => return Err(Error::unsupported(&feature)),
                Statement::Stop => {
                    self.host.log("Stop: VBA execution halted");
                    return Err(Error { fatal: true, ..Error::runtime(-32001, "Stop: VBA execution halted") });
                }
                Statement::Randomize(e) => {
                    let n = if let Some(e) = e { Box::pin(self.eval_scalar(&e)).await?.number()? } else { self.clock.date_serial() };
                    self.rng = n.to_bits() as u32 ^ (n.to_bits() >> 32) as u32;
                }
                _ => return Err(Error::runtime(5, "Invalid compiled statement")),
            },
        }
        self.frame_mut()?.iteration_memory = each.values().flat_map(|state| &state.values).fold(0usize, |n, value| n.saturating_add(value.memory()));
        self.memory()
    }
}
fn object_id(value: Value) -> Result<u64> {
    match value {
        Value::Object(Some(id)) => Ok(id),
        Value::Object(None) => Err(Error::runtime(91, "Object variable not set")),
        _ => Err(mismatch()),
    }
}
fn constant(n: &str) -> Option<Value> {
    Some(Value::Long(match n {
        "vbcrlf" => return Some(Value::String("\r\n".into())),
        "vbcr" => return Some(Value::String("\r".into())),
        "vblf" => return Some(Value::String("\n".into())),
        "vbtab" => return Some(Value::String("\t".into())),
        "vbnullstring" => return Some(Value::String(String::new())),
        "vbbinarycompare" => 0,
        "vbsunday" => 1,
        "vbtextcompare" | "vbok" | "vbokcancel" => 1,
        "vbmonday" => 2,
        "vbyes" => 6,
        "vbno" => 7,
        "vbcritical" => 16,
        "vbinformation" => 64,
        _ => return None,
    }))
}
fn make_frame(module: usize, name: &str, stmts: &[Stmt]) -> Result<Frame> {
    let mut code = Vec::new();
    compile(stmts, &mut code, &mut Vec::new())?;
    let mut with_depth = 0usize;
    for instruction in &mut code {
        instruction.with_depth = with_depth;
        match &instruction.op {
            Op::With(_) => with_depth = with_depth.saturating_add(1),
            Op::EndWith => with_depth = with_depth.saturating_sub(1),
            _ => {}
        }
    }
    let labels = code.iter().enumerate().filter_map(|(i, s)| if let Op::Label(n) = &s.op { Some((n.clone(), i)) } else { None }).collect();
    Ok(Frame {
        module,
        procedure: name.into(),
        vars: BTreeMap::new(),
        code,
        labels,
        pc: 0,
        handler: String::new(),
        handling: false,
        fault: None,
        line: 1,
        with_base: 0,
        iteration_memory: 0,
    })
}
fn emit(code: &mut Vec<Instruction>, line: usize, op: Op) -> usize {
    let i = code.len();
    code.push(Instruction { line, op, with_depth: 0 });
    i
}
fn patch(code: &mut [Instruction], i: usize, dest: usize) {
    if let Some(ins) = code.get_mut(i) {
        match &mut ins.op {
            Op::Jump(d) | Op::Unless(_, d) | Op::ForInit(_, _, _, _, d) | Op::EachInit(_, _, d) => *d = dest,
            _ => {}
        }
    }
}
fn compile(stmts: &[Stmt], code: &mut Vec<Instruction>, loops: &mut Vec<(String, Vec<usize>)>) -> Result<()> {
    for stmt in stmts {
        let line = stmt.line;
        match &stmt.kind {
            Statement::If(branches, other) => {
                let mut exits = Vec::new();
                for (test, body) in branches {
                    let i = emit(code, line, Op::Unless(test.clone(), 0));
                    compile(body, code, loops)?;
                    exits.push(emit(code, line, Op::Jump(0)));
                    let dest = code.len();
                    patch(code, i, dest);
                }
                compile(other, code, loops)?;
                let dest = code.len();
                for i in exits {
                    patch(code, i, dest);
                }
            }
            Statement::Select(e, cases, other) => {
                let temp = format!("__select{}", code.len());
                emit(
                    code,
                    line,
                    Op::Simple(Statement::Decl(vec![Decl {
                        public: false,
                        line,
                        name: temp.clone(),
                        ty: "variant".into(),
                        bounds: None,
                        initial: Some(e.clone()),
                        constant: false,
                        persistent: false,
                    }])),
                );
                let mut exits = Vec::new();
                for (tests, body) in cases {
                    let mut condition = Expr::Literal(Value::Boolean(false));
                    for test in tests {
                        let lhs = Box::new(Expr::Name(temp.clone()));
                        let test = match test {
                            Case::Value(e) => Expr::Binary("=".into(), lhs, Box::new(e.clone())),
                            Case::Is(op, e) => Expr::Binary(op.clone(), lhs, Box::new(e.clone())),
                            Case::Range(lo, hi) => Expr::Binary(
                                "and".into(),
                                Box::new(Expr::Binary(">=".into(), lhs.clone(), Box::new(lo.clone()))),
                                Box::new(Expr::Binary("<=".into(), lhs, Box::new(hi.clone()))),
                            ),
                        };
                        condition = Expr::Binary("or".into(), Box::new(condition), Box::new(test));
                    }
                    let i = emit(code, line, Op::Unless(condition, 0));
                    compile(body, code, loops)?;
                    exits.push(emit(code, line, Op::Jump(0)));
                    let dest = code.len();
                    patch(code, i, dest);
                }
                compile(other, code, loops)?;
                let dest = code.len();
                for i in exits {
                    patch(code, i, dest);
                }
            }
            Statement::For(name, start, end, step, body) => {
                let i = emit(code, line, Op::ForInit(name.clone(), start.clone(), end.clone(), step.clone(), 0));
                let top = code.len();
                loops.push(("for".into(), Vec::new()));
                compile(body, code, loops)?;
                emit(code, line, Op::ForNext(name.clone(), top));
                let dest = code.len();
                patch(code, i, dest);
                if let Some((_, exits)) = loops.pop() {
                    for i in exits {
                        patch(code, i, dest);
                    }
                }
            }
            Statement::Each(name, expr, body) => {
                let i = emit(code, line, Op::EachInit(name.clone(), expr.clone(), 0));
                let top = code.len();
                loops.push(("for".into(), Vec::new()));
                compile(body, code, loops)?;
                emit(code, line, Op::EachNext(name.clone(), top));
                let dest = code.len();
                patch(code, i, dest);
                if let Some((_, exits)) = loops.pop() {
                    for i in exits {
                        patch(code, i, dest);
                    }
                }
            }
            Statement::Do(pre, body, post) => {
                let top = code.len();
                let mut skip = None;
                if let Some((until, e)) = pre {
                    let e = if *until { Expr::Unary("not".into(), Box::new(e.clone())) } else { e.clone() };
                    skip = Some(emit(code, line, Op::Unless(e, 0)));
                }
                loops.push(("do".into(), Vec::new()));
                compile(body, code, loops)?;
                if let Some((until, e)) = post {
                    let e = if *until { e.clone() } else { Expr::Unary("not".into(), Box::new(e.clone())) };
                    emit(code, line, Op::Unless(e, top));
                } else {
                    emit(code, line, Op::Jump(top));
                }
                let dest = code.len();
                if let Some(i) = skip {
                    patch(code, i, dest);
                }
                if let Some((_, exits)) = loops.pop() {
                    for i in exits {
                        patch(code, i, dest);
                    }
                }
            }
            Statement::With(e, body) => {
                emit(code, line, Op::With(e.clone()));
                compile(body, code, loops)?;
                emit(code, line, Op::EndWith);
            }
            Statement::Exit(kind) => {
                let i = emit(code, line, Op::Jump(usize::MAX));
                if kind == "for" || kind == "do" {
                    let target = loops.iter_mut().rev().find(|(k, _)| k == kind).ok_or_else(|| Error::runtime(1002, "Exit outside loop"))?;
                    target.1.push(i);
                }
            }
            Statement::Label(label) => {
                emit(code, line, Op::Label(label.clone()));
            }
            s => {
                emit(code, line, Op::Simple(s.clone()));
            }
        }
    }
    Ok(())
}

fn arithmetic_type(op: &str, a: &Value, b: &Value) -> &'static str {
    if ["/", "^"].contains(&op) {
        return "double";
    }
    if matches!((a, b), (Value::Date(_), Value::Date(_))) && op == "-" {
        return "double";
    }
    if matches!((a, b), (Value::Date(_), _) | (_, Value::Date(_))) {
        return if op == "*" { "double" } else { "date" };
    }
    if matches!((a, b), (Value::Currency(_), _) | (_, Value::Currency(_))) {
        if op == "*"
            && matches!(
                (a, b),
                (Value::Single(_) | Value::Double(_) | Value::String(_), _) | (_, Value::Single(_) | Value::Double(_) | Value::String(_))
            )
        {
            return "double";
        }
        return "currency";
    }
    if matches!((a, b), (Value::Double(_) | Value::String(_), _) | (_, Value::Double(_) | Value::String(_))) {
        return "double";
    }
    if matches!((a, b), (Value::Single(_), _) | (_, Value::Single(_))) {
        return if matches!((a, b), (Value::Long(_) | Value::LongLong(_), _) | (_, Value::Long(_) | Value::LongLong(_))) {
            "double"
        } else {
            "single"
        };
    }
    if matches!((a, b), (Value::LongLong(_), _) | (_, Value::LongLong(_))) {
        return "longlong";
    }
    if matches!((a, b), (Value::Long(_), _) | (_, Value::Long(_))) {
        return "long";
    }
    if matches!((a, b), (Value::Byte(_), Value::Byte(_) | Value::Empty) | (Value::Empty, Value::Byte(_))) {
        return "byte";
    }
    "integer"
}

struct YieldOnce(bool);
impl std::future::Future for YieldOnce {
    type Output = ();
    fn poll(mut self: std::pin::Pin<&mut Self>, context: &mut std::task::Context<'_>) -> std::task::Poll<()> {
        if self.0 {
            std::task::Poll::Ready(())
        } else {
            self.0 = true;
            context.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    }
}

/// Returns control to an embedding frame loop without requiring an executor dependency.
pub async fn yield_once() {
    YieldOnce(false).await;
}

#[cfg(test)]
mod array_memory_tests {
    use super::*;

    #[test]
    fn indexed_writes_keep_shared_slot_memory_exact() {
        let p = place(Value::Array(Array::new(vec![(1, 2)], "variant".into(), false, 10).unwrap()), "variant".into(), false);
        let mut element = p.clone();
        element.indices.push(1);
        for value in [Value::String("longer".repeat(100)), Value::String("x".into()), Value::Empty] {
            element.write(value).unwrap();
            assert_eq!(p.slot.borrow().memory, p.slot.borrow().value.memory());
        }
        let before = p.slot.borrow().memory;
        let mut invalid = p.clone();
        invalid.indices.push(3);
        assert!(invalid.write(Value::String("bad".into())).is_err());
        assert_eq!(p.slot.borrow().memory, before);
        p.write(Value::String("replacement".into())).unwrap();
        assert_eq!(p.slot.borrow().memory, p.slot.borrow().value.memory());
    }

    #[test]
    fn indexed_array_reads_preserve_copies_and_byref_elements() {
        let program = Program::parse(
            "Test",
            "Function Main()\nDim a(1 To 2, 1 To 2) As Long\nDim b As Variant\na(1,1) = 7\nb = a\nChange a(1,1)\na(2,2) = b(1,1) + a(1,1)\nMain = a(2,2)\nEnd Function\nSub Change(ByRef target As Long)\ntarget = target + 1\nEnd Sub",
        )
        .unwrap();
        let result = Interpreter::new(&program, crate::NoHost, crate::FixedClock, Limits::default()).run("Test", "Main", &[]).unwrap();
        assert_eq!(result.number().unwrap(), 15.0);
    }

    #[test]
    fn indexed_array_growth_still_exceeds_interpreter_memory_limit() {
        let program = Program::parse(
            "Test",
            "Sub Main()\nDim a(1 To 30) As Variant\nDim i As Long\nFor i = 1 To 30\na(i) = String(1000, \"x\")\nNext i\nEnd Sub",
        )
        .unwrap();
        let limits = Limits { max_memory_bytes: 16_000, ..Limits::default() };
        let error = Interpreter::new(&program, crate::NoHost, crate::FixedClock, limits).run("Test", "Main", &[]).unwrap_err();
        assert!(error.fatal);
        assert!(error.description.contains("memory"));
    }
}
