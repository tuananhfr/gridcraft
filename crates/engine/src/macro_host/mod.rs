//! Explicit, consent-gated macro execution. Opening documents never starts a run or fires events.
mod host;
mod udf;
use crate::{EngineError, HistoryEntry, Result, Session};
use gridcraft_model::Workbook;
use gridcraft_vbarun::{Clock, Error, Interpreter, Limits, Program};
use serde_json::{Value as Json, json};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll, Waker},
};

#[derive(Clone, Debug, Default)]
pub struct MacroState {
    pub consent: bool,
    pub log: Vec<Json>,
    pub cancellation: Arc<AtomicBool>,
    pub running: Arc<AtomicBool>,
}
#[derive(Clone, Debug)]
pub struct MacroOptions {
    pub wall_ms: u64,
    pub max_steps: u64,
    pub max_memory_bytes: usize,
    pub instructions_per_frame: usize,
}
impl Default for MacroOptions {
    fn default() -> Self {
        Self {
            wall_ms: if cfg!(target_arch = "wasm32") { 5_000 } else { 30_000 },
            max_steps: 1_000_000,
            max_memory_bytes: 32 * 1024 * 1024,
            instructions_per_frame: 256,
        }
    }
}
struct RunOutput {
    host: host::ExcelHost,
    result: gridcraft_vbarun::Result<gridcraft_vbarun::Value>,
    steps: u64,
}
#[derive(Default)]
pub(crate) struct MacroInteraction {
    request: Option<(String, Json)>,
    response: Option<Json>,
}
type Interaction = std::rc::Rc<std::cell::RefCell<MacroInteraction>>;

pub struct MacroTask {
    interaction: Option<Interaction>,
    future: Pin<Box<dyn Future<Output = RunOutput>>>,
    uid: u64,
    original: Arc<Workbook>,
    running: Arc<AtomicBool>,
    cancellation: Arc<AtomicBool>,
    log_index: usize,
    started: u64,
    finished: bool,
}
impl MacroTask {
    pub fn cancel(&self) {
        self.cancellation.store(true, Ordering::Relaxed);
    }
    pub fn take_prompt(&mut self) -> Option<(String, Json)> {
        self.interaction.as_ref()?.borrow_mut().request.take()
    }
    pub fn answer_prompt(&mut self, response: Json) {
        if let Some(interaction) = &self.interaction {
            interaction.borrow_mut().response = Some(response);
        }
    }
}
impl Drop for MacroTask {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Relaxed);
        self.running.store(false, Ordering::Relaxed);
    }
}

#[derive(Clone)]
struct RunClock {
    #[cfg(not(target_arch = "wasm32"))]
    start: std::time::Instant,
    epoch: f64,
}
impl RunClock {
    #[allow(clippy::disallowed_methods)] // Instant is constructed only on native targets.
    fn new() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            start: std::time::Instant::now(),
            epoch: gridcraft_core::date::unix_now_ms().unwrap_or(0.0),
        }
    }
}
impl Clock for RunClock {
    fn monotonic_ms(&self) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
        }
        #[cfg(target_arch = "wasm32")]
        {
            (gridcraft_core::date::unix_now_ms().unwrap_or(self.epoch) - self.epoch).max(0.0) as u64
        }
    }
    fn date_serial(&self) -> f64 {
        gridcraft_core::date::serial_from_unix_ms(self.epoch + self.monotonic_ms() as f64)
    }
}
fn now() -> u64 {
    gridcraft_core::date::unix_now_ms().unwrap_or(0.0).max(0.0) as u64
}
fn failure(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}

fn execution_program(project: &gridcraft_vba::Project) -> gridcraft_vbarun::Result<Program> {
    let mut modules = Vec::new();
    for module in &project.modules {
        if let Ok(parsed) = gridcraft_vbarun::Module::parse(&module.name, &module.source) {
            modules.push(parsed);
            continue;
        }
        let mut lines: Vec<String> = module.source.lines().map(str::to_string).collect();
        for procedure in module.procedures.iter().filter(|p| p.compatibility.level > 1) {
            let start = (procedure.line as usize).saturating_sub(1);
            let end = lines
                .iter()
                .enumerate()
                .skip(start.saturating_add(1))
                .find(|(_, line)| {
                    let line = line.trim().to_ascii_lowercase();
                    line.starts_with("end sub") || line.starts_with("end function") || line.starts_with("end property")
                })
                .map(|(i, _)| i);
            let Some(end) = end else {
                return Err(Error::runtime(1002, "Unsupported procedure has no terminator"));
            };
            // Dormant platform code must not prevent compiling a safe procedure in the same project.
            for line in lines.get_mut(start..=end).into_iter().flatten() {
                line.clear();
            }
            if let Some(line) = lines.get_mut(start) {
                *line = format!("Public Sub {}()", procedure.name);
            }
            if let Some(line) = lines.get_mut(end) {
                *line = "End Sub".into();
            }
        }
        let mut declaration = false;
        for line in &mut lines {
            let text = line.trim().to_ascii_lowercase();
            let text = text.strip_prefix("public ").or_else(|| text.strip_prefix("private ")).unwrap_or(&text);
            if declaration || text.starts_with("declare ") {
                declaration = text.trim_end().ends_with('_');
                line.clear();
            } else if text == "option private module" {
                line.clear();
            }
        }
        modules.push(gridcraft_vbarun::Module::parse(&module.name, &lines.join("\n"))?);
    }
    Ok(Program { modules })
}

impl Session {
    /// Starts a suspended transaction; the UI calls `step_macro` once per frame.
    pub fn start_macro(&mut self, name: &str, options: MacroOptions) -> Result<MacroTask> {
        self.start_macro_internal(name, options, false)
    }
    pub fn start_macro_interactive(&mut self, name: &str, options: MacroOptions) -> Result<MacroTask> {
        self.start_macro_internal(name, options, true)
    }
    fn start_macro_internal(&mut self, name: &str, options: MacroOptions, _interactive: bool) -> Result<MacroTask> {
        let doc = self.doc()?;
        if doc.macro_state.running.load(Ordering::Relaxed) {
            return Err(failure("A macro is already running"));
        }
        let analysis = crate::macros::project(&doc.wb).ok_or_else(|| failure("This document has no VBA project"))?.map_err(failure)?;
        let qualified = name.rsplit('!').next().unwrap_or(name).trim_matches('\'');
        let found: Vec<_> = analysis
            .modules
            .iter()
            .flat_map(|m| m.procedures.iter().map(move |p| (m, p)))
            .filter(|(m, p)| p.name.eq_ignore_ascii_case(qualified) || format!("{}.{}", m.name, p.name).eq_ignore_ascii_case(qualified))
            .collect();
        if found.len() != 1 {
            return Err(failure("Macro name is missing or ambiguous"));
        }
        let (module, procedure) = found.first().copied().ok_or_else(|| failure("Macro not found"))?;
        let module_name = module.name.clone();
        let procedure_name = procedure.name.clone();
        let started = now();
        let clock = RunClock::new();
        let mut entry = json!({"macro":format!("{module_name}.{procedure_name}"),"module":module_name,"procedure":procedure_name,"start":started,"end":null,"steps":0,"cellsChanged":0,"status":"running","events":"notSupported"});
        let shape = module.kind == gridcraft_vba::ModuleKind::Standard
            && !module.private_module
            && procedure.scope == gridcraft_vba::Scope::Public
            && procedure.kind == gridcraft_vba::ProcKind::Sub
            && procedure.params.iter().all(|p| p.optional || p.param_array)
            && procedure.event.is_none();
        let refusal = if procedure.compatibility.level > 1 {
            Some(json!({"message":"This macro compatibility level is not supported","level":procedure.compatibility.level,
                "reasons":procedure.compatibility.reasons,"module":module.name,"line":procedure.compatibility.reasons.first().map(|r| r.line).unwrap_or(procedure.line)}))
        } else if !shape {
            Some(
                json!({"message":"Only public level-1 procedures without required parameters can run; events are not run automatically","module":module.name,"line":procedure.line}),
            )
        } else {
            None
        };
        if let Some(error) = refusal {
            entry["status"] = json!("refused");
            entry["end"] = json!(now());
            entry["error"] = error.clone();
            entry["rolledBack"] = json!(true);
            self.push_macro_log(entry)?;
            return Err(failure(error.to_string()));
        }
        if !doc.macro_state.consent {
            self.ui_requests.push(crate::UiRequest::Dialog(
                "macro.consent".into(),
                json!({"macro":name,"level":1,"canChangeCells":true,"sessionOnly":true,"event":procedure.event}),
            ));
            return Err(failure("Macro consent is required for this document session"));
        }
        let program = match execution_program(&analysis) {
            Ok(program) => program,
            Err(error) => {
                entry["status"] = json!("failed");
                entry["end"] = json!(now());
                entry["rolledBack"] = json!(true);
                entry["error"] = json!({"module":error.module,"line":error.line,"message":error.description});
                self.push_macro_log(entry)?;
                return Err(failure(error.to_string()));
            }
        };
        let mut isolated = Session::new();
        isolated.region = self.region;
        isolated.prefs = self.prefs.clone();
        isolated.clipboard = self.clipboard.clone();
        let mut copied = doc.clone();
        copied.undo.clear();
        copied.redo.clear();
        copied.macro_state = MacroState::default();
        isolated.add_document(copied)?;
        isolated.execution_depth = 1;
        let original = doc.wb.clone();
        let uid = doc.uid;
        let cancellation = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let doc = self.doc_mut()?;
        doc.macro_state.cancellation = cancellation.clone();
        doc.macro_state.running = running.clone();
        let log_index = self.push_macro_log(entry)?;
        let limits = Limits {
            max_steps: options.max_steps.clamp(1, 10_000_000),
            max_memory_bytes: options.max_memory_bytes.clamp(1024, 128 * 1024 * 1024),
            deadline_ms: Some(options.wall_ms.clamp(1, 300_000)),
            cancellation: cancellation.clone(),
            ..Limits::default()
        };
        let interaction = _interactive.then(Interaction::default);
        let host_interaction = interaction.clone();
        let future = Box::pin(async move {
            let mut host = host::ExcelHost::new(isolated);
            host.set_analysis(analysis);
            host.interaction = host_interaction;
            host.set_budget(limits.cancellation.clone(), options.wall_ms.clamp(1, 300_000), limits.max_memory_bytes, clock.clone());
            let mut interpreter = Interpreter::new(&program, host, clock, limits);
            interpreter.set_quantum(options.instructions_per_frame);
            let result = interpreter.run_async(&module_name, &procedure_name, &[]).await;
            let steps = interpreter.steps();
            RunOutput { host: interpreter.host, result, steps }
        });
        Ok(MacroTask { interaction, future, uid, original, running, cancellation, log_index, started, finished: false })
    }
    fn push_macro_log(&mut self, entry: Json) -> Result<usize> {
        let state = &mut self.doc_mut()?.macro_state;
        if state.log.len() >= 100 {
            state.log.remove(0);
        }
        state.log.push(entry);
        Ok(state.log.len().saturating_sub(1))
    }
    /// `None` means the transaction is still suspended. An error always discards all edits.
    pub fn step_macro(&mut self, task: &mut MacroTask) -> Result<Option<Json>> {
        if task.finished {
            return Err(failure("Macro task has already finished"));
        }
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let polled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task.future.as_mut().poll(&mut context)));
        let polled = match polled {
            Ok(polled) => polled,
            Err(_) => {
                task.finished = true;
                task.running.store(false, Ordering::Relaxed);
                let summary = json!({"status":"failed","rolledBack":true,"start":task.started,"end":now(),"cellsChanged":0,
                    "error":{"message":"Macro execution failed unexpectedly","line":1}});
                if let Some(doc) = self.docs.iter_mut().find(|d| d.uid == task.uid)
                    && let Some(entry) = doc.macro_state.log.get_mut(task.log_index)
                    && let Some(map) = summary.as_object()
                {
                    for (key, value) in map {
                        entry[key] = value.clone();
                    }
                }
                return Ok(Some(summary));
            }
        };
        let output = match polled {
            Poll::Pending => return Ok(None),
            Poll::Ready(output) => output,
        };
        task.finished = true;
        task.running.store(false, Ordering::Relaxed);
        let index = self.docs.iter().position(|d| d.uid == task.uid).ok_or_else(|| failure("Macro document was closed; edits discarded"))?;
        let unchanged = self.docs.get(index).is_some_and(|d| Arc::ptr_eq(&d.wb, &task.original));
        let result = if !unchanged {
            Err(Error::runtime(1004, "Document changed during execution; macro edits discarded"))
        } else if task.cancellation.load(Ordering::Relaxed) {
            Err(Error { fatal: true, ..Error::runtime(-32000, "VBA sandbox limit: cancelled") })
        } else {
            output.result
        };
        let status = if result.is_ok() {
            "completed"
        } else if task.cancellation.load(Ordering::Relaxed) {
            "cancelled"
        } else {
            "failed"
        };
        let count = output
            .host
            .changed
            .iter()
            .filter(|(sheet, at)| {
                let old = task.original.sheet(*sheet).and_then(|s| s.cell(*at));
                let new = output.host.session.doc().ok().and_then(|d| d.wb.sheet(*sheet)).and_then(|s| s.cell(*at));
                old != new
            })
            .count();
        let error =
            result.as_ref().err().map(|e| json!({"number":e.number,"message":e.description,"module":e.module,"procedure":e.procedure,"line":e.line}));
        let completed = now();
        let summary = json!({"status":status,"steps":output.steps,"cellsChanged":if result.is_ok() {count} else {0},
            "attemptedCellsChanged":count,"error":error,"rolledBack":result.is_err(),"start":task.started,"end":completed,"messages":output.host.messages});
        let timestamp = self.history_clock.map(|clock| clock()).unwrap_or(completed);
        let grouped = self.execution_depth > 1;
        if let Some(doc) = self.docs.get_mut(index) {
            if let Some(entry) = doc.macro_state.log.get_mut(task.log_index)
                && let Some(map) = summary.as_object()
            {
                for (key, value) in map {
                    entry[key] = value.clone();
                }
            }
            if result.is_ok() {
                let changed = output.host.session.doc().map(|d| !Arc::ptr_eq(&d.wb, &task.original)).unwrap_or(false);
                if changed {
                    let isolated_doc = output.host.session.doc()?;
                    let transaction_id = doc.next_transaction_id;
                    let label =
                        doc.macro_state.log.get(task.log_index).and_then(|e| e.get("macro")).and_then(Json::as_str).unwrap_or("Macro").to_string();
                    if !grouped {
                        doc.undo.push(HistoryEntry {
                            transaction_id,
                            timestamp,
                            label,
                            wb: doc.wb.clone(),
                            selection: doc.selection.clone(),
                            stale: doc.calc.stale.clone(),
                        });
                        if doc.undo.len() > crate::UNDO_LIMIT {
                            doc.undo.remove(0);
                        }
                        doc.redo.clear();
                    }
                    doc.wb = isolated_doc.wb.clone();
                    doc.calc = isolated_doc.calc.clone();
                    doc.selection = isolated_doc.selection.clone();
                    doc.sheet_selections = isolated_doc.sheet_selections.clone();
                    doc.revision = doc.revision.saturating_add(1);
                    if !grouped {
                        doc.next_transaction_id = transaction_id.saturating_add(1);
                        self.journal_metadata.truncate(self.journal.len());
                        self.journal.push(("macros.run".into(), json!({"name":doc.macro_state.log.get(task.log_index).and_then(|e|e.get("macro"))})));
                        self.journal_metadata.push((transaction_id, timestamp));
                        if self.journal.len() > 10_000 {
                            self.journal.drain(..5_000);
                            self.journal_metadata.drain(..5_000);
                        }
                    }
                    self.last_repeatable = None;
                }
            }
        }
        if result.is_ok() {
            self.clipboard = output.host.session.clipboard.clone();
        }
        self.ui_requests.extend(output.host.session.ui_requests);
        Ok(Some(summary))
    }
}
pub(crate) fn consent(session: &mut Session, params: &Json) -> Result<Json> {
    let doc = session.doc_mut()?;
    let state = &mut doc.macro_state;
    if let Some(allow) = params.get("allow").and_then(Json::as_bool) {
        state.consent = allow;
        doc.calc.external = allow.then(|| Arc::new(udf::Functions::new(&doc.wb)) as Arc<dyn gridcraft_calc::eval::ExternalFunctions>);
        if !allow {
            state.cancellation.store(true, Ordering::Relaxed);
        }
    }
    let allowed = state.consent;
    if params.get("allow").and_then(Json::as_bool).is_some() {
        doc.calc.recalc_all(Arc::make_mut(&mut doc.wb));
    }
    Ok(json!({"consent":allowed,"persisted":false,"events":"notSupported"}))
}
pub(crate) fn stop(session: &mut Session, _: &Json) -> Result<Json> {
    let state = &session.doc()?.macro_state;
    state.cancellation.store(true, Ordering::Relaxed);
    Ok(json!({"stopping":state.running.load(Ordering::Relaxed)}))
}
pub(crate) fn log(session: &mut Session, _: &Json) -> Result<Json> {
    let state = &mut session.doc_mut()?.macro_state;
    if !state.running.load(Ordering::Relaxed)
        && let Some(entry) = state.log.last_mut()
        && entry.get("status").and_then(Json::as_str) == Some("running")
    {
        entry["status"] = json!("cancelled");
        entry["rolledBack"] = json!(true);
        entry["end"] = json!(now());
    }
    Ok(json!({"runs":session.doc()?.macro_state.log,"running":session.doc()?.macro_state.running.load(Ordering::Relaxed)}))
}
pub(crate) fn run(session: &mut Session, params: &Json) -> Result<Json> {
    let name = params.get("name").and_then(Json::as_str).ok_or_else(|| failure("macros.run requires name"))?;
    let mut options = MacroOptions::default();
    if let Some(ms) = params.get("wallMs").and_then(Json::as_u64) {
        options.wall_ms = ms;
    }
    if let Some(steps) = params.get("maxSteps").and_then(Json::as_u64) {
        options.max_steps = steps;
    }
    let mut task = session.start_macro(name, options)?;
    loop {
        if let Some(result) = session.step_macro(&mut task)? {
            return Ok(result);
        }
    }
}
