use gridcraft_vbarun::{FixedClock, Interpreter, Limits, NoHost, Program, Value};
use std::{
    future::Future,
    task::{Context, Poll, Waker},
};

#[test]
fn cooperative_nested_calls_preserve_locals_and_byref() {
    let program=Program::parse("Test","Dim total As Long\nSub Main()\nDim i As Long\nFor i=1 To 100\nAdd total, i\nNext i\nEnd Sub\nSub Add(ByRef target As Long, ByVal n As Long)\ntarget=target+n\nEnd Sub").unwrap();
    let mut vm = Interpreter::new(&program, NoHost, FixedClock, Limits::default());
    vm.set_quantum(3);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut pending = 0;
    {
        let mut future = Box::pin(vm.run_async("Test", "Main", &[]));
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Pending => pending += 1,
                Poll::Ready(result) => {
                    result.unwrap();
                    break;
                }
            }
        }
    }
    assert!(pending > 50);
    assert_eq!(vm.global_value("Test", "total"), Some(Value::Long(5050)));
}
#[test]
fn cancellation_after_suspend_bypasses_error_handler() {
    let program = Program::parse("Test", "Sub Main()\nOn Error Resume Next\nDo\nLoop\nEnd Sub").unwrap();
    let limits = Limits::default();
    let cancellation = limits.cancellation.clone();
    let mut vm = Interpreter::new(&program, NoHost, FixedClock, limits);
    vm.set_quantum(2);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = Box::pin(vm.run_async("Test", "Main", &[]));
    assert!(matches!(future.as_mut().poll(&mut context), Poll::Pending));
    cancellation.store(true, std::sync::atomic::Ordering::Relaxed);
    let Poll::Ready(Err(error)) = future.as_mut().poll(&mut context) else { panic!("expected fatal cancellation") };
    assert!(error.fatal);
    assert!(error.description.contains("cancelled"));
}
