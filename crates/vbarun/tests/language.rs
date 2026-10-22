use gridcraft_vbarun::{Argument, Clock, Error, FixedClock, Host, Interpreter, Limits, NoHost, Program, Value};

fn evaluate(body: &str) -> Value {
    let source = format!("Function Test()\n{body}\nEnd Function");
    let program = Program::parse("Generic", &source).unwrap();
    Interpreter::new(&program, NoHost, FixedClock, Limits::default()).run("Generic", "Test", &[]).unwrap()
}
fn number(value: Value) -> f64 {
    value.number().unwrap()
}
#[test]
fn arithmetic_and_intrinsic_table() {
    for (expression, expected) in [
        ("2 + 3 * 4", 14.0),
        ("-2 ^ 2", -4.0),
        ("2 ^ 3 ^ 2", 64.0),
        ("7 \\ 2", 3.0),
        ("7.5 \\ 2", 4.0),
        ("-7 Mod 3", -1.0),
        ("CInt(2.5)", 2.0),
        ("CLng(3.5)", 4.0),
        ("Round(2.5)", 2.0),
        ("Int(-1.5)", -2.0),
        ("Fix(-1.5)", -1.0),
        ("Abs(-5)", 5.0),
        ("Sqr(16)", 4.0),
        ("Sgn(0)", 0.0),
        ("Len(\"abc\")", 3.0),
        ("InStr(\"abc\", \"b\")", 2.0),
        ("InStrRev(\"aba\", \"a\")", 3.0),
        ("Asc(\"A\")", 65.0),
        ("Val(\"12.5kg\")", 12.5),
        ("Year(DateSerial(2024, 2, 29))", 2024.0),
        ("Month(DateAdd(\"m\", 1, #2024-01-31#))", 2.0),
        ("DateDiff(\"d\", #2024-01-01#, #2024-01-03#)", 2.0),
        ("LBound(Array(1,2))", 0.0),
        ("UBound(Array(1,2))", 1.0),
    ] {
        assert_eq!(number(evaluate(&format!("Test = {expression}"))), expected, "{expression}");
    }
}
#[test]
fn strings_and_inspection_table() {
    for (expression, expected) in [
        ("Left(\"abcdef\", 2)", "ab"),
        ("Right(\"abcdef\", 2)", "ef"),
        ("Mid(\"abcdef\", 2, 3)", "bcd"),
        ("UCase(\"ab\")", "AB"),
        ("LCase(\"AB\")", "ab"),
        ("Trim(\" a \" )", "a"),
        ("LTrim(\" a \" )", "a "),
        ("RTrim(\" a \" )", " a"),
        ("Space(2)", "  "),
        ("String(3, \"x\")", "xxx"),
        ("StrReverse(\"abc\")", "cba"),
        ("Replace(\"ababa\", \"a\", \"x\")", "xbxbx"),
        ("Join(Split(\"a,b\", \",\"), \"-\")", "a-b"),
        ("ChrW(65)", "A"),
        ("Format(12.5, \"0.00\")", "12.50"),
        ("Format(#2024-01-02#, \"yyyy-mm-dd\")", "2024-01-02"),
        ("TypeName(CByte(1))", "Byte"),
        ("CStr(IIf(True, 1, 2))", "1"),
        ("Choose(2, \"a\", \"b\")", "b"),
        ("Switch(False, \"a\", True, \"b\")", "b"),
    ] {
        assert_eq!(evaluate(&format!("Test = {expression}")), Value::String(expected.into()), "{expression}");
    }
}
#[test]
fn control_flow_table() {
    for (body, expected) in [
        ("If True Then Test = 1 Else Test = 2", 1.0),
        ("If False Then\nTest = 1\nElseIf True Then\nTest = 2\nElse\nTest = 3\nEnd If", 2.0),
        ("For i = 3 To 1 Step -1\nTest = Test + i\nNext i", 6.0),
        ("For i = 1 To 2\nFor j = 1 To 3\nTest = Test + 1\nExit For\nNext\nNext", 2.0),
        ("For Each i In Array(1, 2, 3)\nTest = Test + i\nNext", 6.0),
        ("Do While Test < 3\nTest = Test + 1\nLoop", 3.0),
        ("Do\nTest = Test + 1\nLoop Until Test = 3", 3.0),
        ("While Test < 3\nTest = Test + 1\nWend", 3.0),
        ("Select Case 4\nCase 1, 2\nTest = 1\nCase 3 To 5\nTest = 2\nCase Is > 9\nTest = 3\nCase Else\nTest = 4\nEnd Select", 2.0),
        ("GoTo done\nTest = 1\ndone:\nTest = Test + 2", 2.0),
        ("GoTo 20\n10 Test = 1\n20 Test = Test + 2", 2.0),
        ("Test = 1: Test = Test + _\n 2", 3.0),
    ] {
        assert_eq!(number(evaluate(body)), expected, "{body}");
    }
}
#[test]
fn error_handlers_and_codes() {
    for (expression, expected) in
        [("1 / 0", 11.0), ("CInt(40000)", 6.0), ("CDbl(\"bad\")", 13.0), ("Array(1)(5)", 9.0), ("Shell(\"anything\")", 445.0)]
    {
        assert_eq!(number(evaluate(&format!("On Error Resume Next\nx = {expression}\nTest = Err.Number"))), expected, "{expression}");
    }
    assert_eq!(number(evaluate("On Error GoTo fail\nx = 1 / 0\nTest = 7\nExit Function\nfail:\nResume Next")), 7.0);
    assert_eq!(number(evaluate("On Error Resume Next\nErr.Raise 42, \"generic\", \"failure\"\nTest = Err.Number\nErr.Clear")), 42.0);
}
#[test]
fn arrays_byref_paramarray_optional_and_statics() {
    let source = "Option Base 1\nFunction Test()\nDim a(1 To 2) As Long\na(1) = 4\nBump a(1)\nTest = a(1) + SumArgs(1,2,3) + MissingArg()\nEnd Function\nSub Bump(ByRef x As Long)\nx = x + 1\nEnd Sub\nFunction SumArgs(ParamArray a() As Variant)\nFor Each x In a\nSumArgs = SumArgs + x\nNext\nEnd Function\nFunction MissingArg(Optional x)\nIf IsMissing(x) Then MissingArg = 10\nEnd Function";
    let p = Program::parse("Generic", source).unwrap();
    assert_eq!(number(Interpreter::new(&p, NoHost, FixedClock, Limits::default()).run("Generic", "Test", &[]).unwrap()), 21.0);
    assert_eq!(number(evaluate("Dim a() As Long\nReDim a(1 To 2)\na(1) = 4\nReDim Preserve a(1 To 3)\nTest = a(1) + UBound(a)\nErase a")), 7.0);
}
#[test]
fn compare_text_like_and_mid_statement() {
    let p =
        Program::parse("Generic", "Option Compare Text\nFunction Test()\nTest = \"HELLO\" = \"hello\" And \"abc5\" Like \"a*[0-9]\"\nEnd Function")
            .unwrap();
    assert_eq!(Interpreter::new(&p, NoHost, FixedClock, Limits::default()).run("Generic", "Test", &[]).unwrap(), Value::Boolean(true));
    assert_eq!(evaluate("Dim s As String\ns = \"abcde\"\nMid(s, 2, 2) = \"XY\"\nTest = s"), Value::String("aXYde".into()));
}
#[derive(Default)]
struct FakeHost {
    log: Vec<String>,
    calls: Vec<Vec<Argument>>,
}
impl Host for FakeHost {
    fn global(&mut self, name: &str) -> Option<Value> {
        (name == "widget").then_some(Value::Object(Some(7)))
    }
    fn get_member(&mut self, obj: u64, name: &str, args: &[Argument]) -> gridcraft_vbarun::Result<Value> {
        assert_eq!(obj, 7);
        assert_eq!(name, "value");
        self.calls.push(args.to_vec());
        Ok(Value::Long(12))
    }
    fn set_member(&mut self, obj: u64, _name: &str, args: &[Argument], value: Value) -> gridcraft_vbarun::Result<()> {
        assert_eq!(obj, 7);
        self.calls.push(args.to_vec());
        assert_eq!(number(value), 3.0);
        Ok(())
    }
    fn log(&mut self, s: &str) {
        self.log.push(s.into());
    }
}
#[test]
fn host_named_omitted_arguments_with_and_logging() {
    let p = Program::parse("Generic", "Sub Test()\nWith widget\nx = .Value(, index:=2)\n.Value = 3\nEnd With\nDebug.Print x\nEnd Sub").unwrap();
    let mut vm = Interpreter::new(&p, FakeHost::default(), FixedClock, Limits::default());
    vm.run("Generic", "Test", &[]).unwrap();
    assert_eq!(vm.host.log, ["12"]);
    assert_eq!(vm.host.calls.first().unwrap().get(1).unwrap().name.as_deref(), Some("index"));
    assert_eq!(vm.host.calls.first().unwrap().first().unwrap().value, Value::Missing);
}
#[test]
fn fatal_limits_bypass_on_error_and_cancellation() {
    let p = Program::parse("Generic", "Sub Test()\nOn Error Resume Next\nDo\nx = x + 1\nLoop\nEnd Sub").unwrap();
    let limits = Limits { max_steps: 100, ..Limits::default() };
    let e = Interpreter::new(&p, NoHost, FixedClock, limits.clone()).run("Generic", "Test", &[]).unwrap_err();
    assert!(e.fatal);
    assert_eq!(e.module, "Generic");
    assert_eq!(e.procedure, "test");
    assert!(e.line >= 3);
    limits.cancellation.store(true, std::sync::atomic::Ordering::Relaxed);
    let e = Interpreter::new(&p, NoHost, FixedClock, limits).run("Generic", "Test", &[]).unwrap_err();
    assert!(e.fatal);
    assert!(e.description.contains("cancelled"));
    let p = Program::parse("Generic", "Sub Test()\nOn Error Resume Next\nTest\nEnd Sub").unwrap();
    let e = Interpreter::new(&p, NoHost, FixedClock, Limits { max_call_depth: 8, ..Limits::default() }).run("Generic", "Test", &[]).unwrap_err();
    assert!(e.fatal);
    assert!(e.description.contains("call depth"));
}
struct FakeClock;
impl Clock for FakeClock {
    fn monotonic_ms(&self) -> u64 {
        100
    }
    fn date_serial(&self) -> f64 {
        45292.5
    }
}
#[test]
fn injected_clock_and_deadline() {
    let p = Program::parse("Generic", "Function Test()\nTest = Timer\nEnd Function").unwrap();
    assert_eq!(number(Interpreter::new(&p, NoHost, FakeClock, Limits::default()).run("Generic", "Test", &[]).unwrap()), 43200.0);
    let e = Interpreter::new(&p, NoHost, FakeClock, Limits { deadline_ms: Some(99), ..Limits::default() }).run("Generic", "Test", &[]).unwrap_err();
    assert!(e.fatal);
}
#[test]
fn unsupported_features_and_parse_limits() {
    assert!(Program::parse("Generic", "Sub Test()\nDim x As New Widget\nEnd Sub").unwrap_err().description.contains("As New"));
    let p = Program::parse("Generic", "Sub Test()\nCreateObject \"Generic.Widget\"\nEnd Sub").unwrap();
    let e: Error = Interpreter::new(&p, NoHost, FixedClock, Limits::default()).run("Generic", "Test", &[]).unwrap_err();
    assert_eq!(e.number, 445);
    assert!(e.description.contains("no disk/network/system access"));
}
