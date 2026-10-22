//! Sandboxed source interpreter for the implementation-independent MS-VBAL language.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod intrinsic;
mod lexer;
mod parser;
mod runtime;
mod value;

pub use parser::{Module, Program};
pub use runtime::{Interpreter, yield_once};
use std::sync::{Arc, atomic::AtomicBool};
pub use value::{Array, Value};

pub type Result<T> = std::result::Result<T, Error>;
pub type Object = u64;

#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub number: i32,
    pub description: String,
    pub module: String,
    pub procedure: String,
    pub source: String,
    pub line: usize,
    /// Resource exhaustion and cancellation bypass VBA error handlers.
    pub fatal: bool,
}
impl Error {
    pub fn runtime(number: i32, description: impl Into<String>) -> Self {
        Self {
            number,
            description: description.into(),
            module: String::new(),
            procedure: String::new(),
            source: String::new(),
            line: 1,
            fatal: false,
        }
    }
    pub fn unsupported(feature: &str) -> Self {
        Self::runtime(445, format!("{feature} not supported in GridCraft (no disk/network/system access)"))
    }
    pub(crate) fn limit(name: &str) -> Self {
        Self { fatal: true, ..Self::runtime(-32000, format!("VBA sandbox limit: {name}")) }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}: error {}: {}", self.module, self.procedure, self.line, self.number, self.description)
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Debug, PartialEq)]
pub struct Argument {
    pub name: Option<String>,
    pub value: Value,
}
impl Argument {
    pub fn positional(value: Value) -> Self {
        Self { name: None, value }
    }
}

/// All external effects are explicitly delegated; the default host grants no capabilities.
#[allow(async_fn_in_trait)]
pub trait Host {
    async fn set_member_async(&mut self, obj: Object, name: &str, args: &[Argument], value: Value) -> Result<()> {
        self.set_member(obj, name, args, value)
    }
    fn scalar(&mut self, value: Value) -> Result<Value> {
        Ok(value)
    }
    fn enter_procedure(&mut self, _module: &str, _procedure: &str) -> Result<()> {
        Ok(())
    }
    fn get_member(&mut self, _obj: Object, name: &str, _args: &[Argument]) -> Result<Value> {
        Err(Error::unsupported(name))
    }
    fn set_member(&mut self, _obj: Object, name: &str, _args: &[Argument], _value: Value) -> Result<()> {
        Err(Error::unsupported(name))
    }
    fn call_method(&mut self, obj: Object, name: &str, args: &[Argument]) -> Result<Value> {
        self.get_member(obj, name, args)
    }
    fn global(&mut self, _name: &str) -> Option<Value> {
        None
    }
    fn msg_box(&mut self, _args: &[Argument]) -> Result<Value> {
        Err(Error::unsupported("MsgBox"))
    }
    fn input_box(&mut self, _args: &[Argument]) -> Result<Value> {
        Err(Error::unsupported("InputBox"))
    }
    /// `existing` distinguishes GetObject from CreateObject, including omitted arguments.
    fn create_object(&mut self, existing: bool, _args: &[Argument]) -> Result<Value> {
        Err(Error::unsupported(if existing { "GetObject" } else { "CreateObject" }))
    }
    fn enumerate(&mut self, _obj: Object) -> Result<Vec<Value>> {
        Err(Error::unsupported("For Each"))
    }
    fn log(&mut self, _message: &str) {}
}
#[derive(Default)]
pub struct NoHost;
impl Host for NoHost {}

/// Both values are supplied by the embedding application; never reads an OS or browser clock.
pub trait Clock {
    fn monotonic_ms(&self) -> u64;
    /// OLE Automation date: day zero is 1899-12-30, with no fictitious 1900 leap day.
    fn date_serial(&self) -> f64;
}
#[derive(Default)]
pub struct FixedClock;
impl Clock for FixedClock {
    fn monotonic_ms(&self) -> u64 {
        0
    }
    fn date_serial(&self) -> f64 {
        0.0
    }
}

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_steps: u64,
    pub max_call_depth: usize,
    pub max_string_length: usize,
    pub max_array_elements: usize,
    pub max_memory_bytes: usize,
    pub deadline_ms: Option<u64>,
    pub cancellation: Arc<AtomicBool>,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_steps: 1_000_000,
            max_call_depth: 64,
            max_string_length: 1_000_000,
            max_array_elements: 100_000,
            max_memory_bytes: 32 * 1024 * 1024,
            deadline_ms: None,
            cancellation: Arc::new(AtomicBool::new(false)),
        }
    }
}
