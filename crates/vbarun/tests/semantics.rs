use gridcraft_vbarun::{Argument, FixedClock, Host, Interpreter, Limits, NoHost, Program, Value};

fn run(source: &str) -> Value {
    let program = Program::parse("Synthetic", source).unwrap();
    Interpreter::new(&program, NoHost, FixedClock, Limits::default()).run("Synthetic", "Test", &[]).unwrap()
}

#[test]
fn declarations_static_const_and_optional_defaults() {
    let source = "Option Explicit\nPrivate total As Long\nPublic Const Factor As Integer = 2\nFunction Count(Optional amount As Long = 3) As Long\nStatic n As Long\nn = n + amount\nCount = n\nEnd Function\nFunction Test()\nDim first As Long\nfirst = Count()\ntotal = Count(1)\nTest = (first + total) * Factor\nEnd Function";
    assert_eq!(run(source).number().unwrap(), 14.0);
}

#[test]
fn property_get_let_and_set() {
    let source = "Private stored As Long\nPrivate reference As Object\nProperty Get Amount() As Long\nAmount = stored\nEnd Property\nProperty Let Amount(ByVal value As Long)\nstored = value\nEnd Property\nProperty Get Item() As Object\nSet Item = reference\nEnd Property\nProperty Set Item(ByVal value As Object)\nSet reference = value\nEnd Property\nFunction Test()\nAmount = 12\nSet Item = Nothing\nIf Item Is Nothing Then Test = Amount\nEnd Function";
    assert_eq!(run(source).number().unwrap(), 12.0);
}

#[test]
fn byval_named_calls_call_and_exit() {
    let source = "Sub Change(ByVal value As Long, Optional extra As Long = 2)\nvalue = value + extra\nExit Sub\nvalue = 999\nEnd Sub\nFunction Add(ByVal first, ByVal second)\nAdd = first + second\nEnd Function\nFunction Test()\nDim value As Long\nvalue = 3\nCall Change(value)\nChange value, extra:=8\nTest = Add(second:=4, first:=value)\nExit Function\nTest = 999\nEnd Function";
    assert_eq!(run(source).number().unwrap(), 7.0);
}

#[test]
fn select_case_under_option_explicit() {
    assert_eq!(
        run("Option Explicit\nFunction Test()\nSelect Case 5\nCase Is < 0\nTest = 1\nCase 2, 4 To 6\nTest = 9\nCase Else\nTest = 0\nEnd Select\nEnd Function")
            .number()
            .unwrap(),
        9.0
    );
}

#[test]
fn loop_variants_and_exits() {
    for body in [
        "Do Until Test = 3\nTest = Test + 1\nLoop",
        "Do\nTest = Test + 1\nLoop While Test < 3",
        "Do\nFor i = 1 To 10\nTest = Test + 1\nIf Test = 3 Then Exit Do\nNext\nLoop",
    ] {
        assert_eq!(run(&format!("Function Test()\n{body}\nEnd Function")).number().unwrap(), 3.0);
    }
}

#[test]
fn resume_retry_label_and_disabled_handler() {
    for body in [
        "On Error GoTo failed\nx = 0\nretry:\nTest = 6 / x\nExit Function\nfailed:\nx = 2\nResume",
        "On Error GoTo failed\nx = 1 / 0\nExit Function\nfailed:\nResume recovered\nrecovered:\nTest = 3",
    ] {
        assert_eq!(run(&format!("Function Test()\n{body}\nEnd Function")).number().unwrap(), 3.0);
    }
    let p = Program::parse("Synthetic", "Sub Test()\nOn Error Resume Next\nx = 1 / 0\nOn Error GoTo 0\nx = 1 / 0\nEnd Sub").unwrap();
    let error = Interpreter::new(&p, NoHost, FixedClock, Limits::default()).run("Synthetic", "Test", &[]).unwrap_err();
    assert_eq!((error.number, error.line), (11, 5));
    assert_eq!((&*error.module, &*error.procedure), ("Synthetic", "test"));
}

#[test]
fn err_source_and_clear() {
    assert_eq!(
        run(
            "Function Test()\nErr.Number = 42\nErr.Source = \"sample\"\nErr.Description = \"message\"\nTest = Err.Source & Err.Description & Err.Number\nEnd Function"
        ),
        Value::String("samplemessage42".into())
    );
    assert_eq!(
        run(
            "Function Test()\nOn Error Resume Next\nErr.Raise 77, \"sample\", \"message\"\nTest = Err.Source & \":\" & Err.Description\nErr.Clear\nTest = Test & \":\" & Err.Number\nEnd Function"
        ),
        Value::String("sample:message:0".into())
    );
}

#[test]
fn conversions_inspection_and_operators() {
    for (expr, expected) in [
        ("CInt(-2.5)", -2.0),
        ("CLng(-3.5)", -4.0),
        ("-7 \\ 3", -2.0),
        ("7 Mod -3", 1.0),
        ("5.5 Mod 2", 0.0),
        ("CDbl(\"2.5\")", 2.5),
        ("CSng(1.5)", 1.5),
        ("CCur(1.2345)", 1.2345),
        ("CByte(255)", 255.0),
        ("CVar(9)", 9.0),
        ("VarType(CLng(2))", 3.0),
        ("Not 0", -1.0),
        ("1 Or 2", 3.0),
        ("3 Xor 1", 2.0),
        ("1 Eqv 1", -1.0),
        ("0 Imp 0", -1.0),
        ("StrComp(\"a\", \"A\", 1)", 0.0),
    ] {
        assert_eq!(run(&format!("Function Test()\nTest = {expr}\nEnd Function")).number().unwrap(), expected, "{expr}");
    }
    for expr in [
        "IsEmpty(Empty)",
        "IsNull(Null)",
        "IsArray(Array())",
        "IsObject(Nothing)",
        "IsNumeric(\"2.5\")",
        "IsDate(\"2024-02-29\")",
        "CBool(1)",
        "\"3\" = 3",
        "\"abc\" <> \"abd\"",
        "\"a\" < \"b\"",
        "2 <= 2",
        "3 >= 2",
    ] {
        assert_eq!(run(&format!("Function Test()\nTest = {expr}\nEnd Function")), Value::Boolean(true), "{expr}");
    }
}

#[test]
fn math_date_and_seeded_random() {
    for (expr, expected) in [
        ("Sin(0)", 0.0),
        ("Cos(0)", 1.0),
        ("Tan(0)", 0.0),
        ("Atn(0)", 0.0),
        ("Exp(0)", 1.0),
        ("Log(1)", 0.0),
        ("Day(CDate(\"2024-02-29\"))", 29.0),
        ("Hour(CDate(\"12:34:56\"))", 12.0),
        ("Minute(CDate(\"12:34:56\"))", 34.0),
        ("Second(CDate(\"12:34:56\"))", 56.0),
        ("Weekday(#2024-01-07#)", 1.0),
        ("DateDiff(\"m\", #2023-12-01#, #2024-02-01#)", 2.0),
    ] {
        assert_eq!(run(&format!("Function Test()\nTest = {expr}\nEnd Function")).number().unwrap(), expected, "{expr}");
    }
    let p = Program::parse("Synthetic", "Function Test()\nRandomize 42\nx = Rnd(-1)\nx = Rnd()\nTest = x = Rnd(0)\nEnd Function").unwrap();
    for _ in 0..2 {
        assert_eq!(Interpreter::new(&p, NoHost, FixedClock, Limits::default()).run("Synthetic", "Test", &[]).unwrap(), Value::Boolean(true));
    }
}

#[test]
fn fixed_array_erase_and_multidimensional_preserve() {
    assert_eq!(run("Function Test()\nDim a() As Long\nReDim Preserve a(1 To 2)\nTest = UBound(a)\nEnd Function").number().unwrap(), 2.0);
    assert!(gridcraft_vbarun::Array::new(vec![(1, 0)], "long".into(), false, 10).is_err());
    assert_eq!(run("Function Test()\nDim a(1 To 2) As Long\na(1) = 8\nErase a\nTest = a(1) + UBound(a)\nEnd Function").number().unwrap(), 2.0);
    assert_eq!(
        run("Function Test()\nDim a() As Long\nReDim a(1 To 2, 1 To 2)\na(2, 1) = 9\nReDim Preserve a(1 To 2, 1 To 3)\nTest = a(2, 1)\nEnd Function")
            .number()
            .unwrap(),
        9.0
    );
}

#[derive(Default)]
struct Objects;
impl Host for Objects {
    fn global(&mut self, name: &str) -> Option<Value> {
        (name == "items").then_some(Value::Object(Some(1)))
    }
    fn enumerate(&mut self, _object: u64) -> gridcraft_vbarun::Result<Vec<Value>> {
        Ok(vec![Value::Long(2), Value::Long(3)])
    }
    fn create_object(&mut self, existing: bool, args: &[Argument]) -> gridcraft_vbarun::Result<Value> {
        assert!(!existing);
        assert_eq!(args.first().unwrap().value, Value::String("Synthetic.Object".into()));
        Ok(Value::Object(Some(1)))
    }
}

#[test]
fn collections_and_creation_host_hook() {
    let p = Program::parse(
        "Synthetic",
        "Function Test()\nSet obj = CreateObject(\"Synthetic.Object\")\nFor Each n In items\nTest = Test + n\nNext\nEnd Function",
    )
    .unwrap();
    assert_eq!(Interpreter::new(&p, Objects, FixedClock, Limits::default()).run("Synthetic", "Test", &[]).unwrap().number().unwrap(), 5.0);
}

#[test]
fn all_denied_features_are_named_and_stable() {
    for feature in ["Shell", "Kill", "Open", "ChDir", "ChDrive", "CurDir", "Environ", "SendKeys", "AppActivate", "CreateObject", "GetObject"] {
        let p = Program::parse("Synthetic", &format!("Sub Test()\n{feature}(\"synthetic\")\nEnd Sub")).unwrap();
        let error = Interpreter::new(&p, NoHost, FixedClock, Limits::default()).run("Synthetic", "Test", &[]).unwrap_err();
        assert_eq!(error.number, 445, "{feature}");
        assert!(error.description.to_lowercase().contains(&feature.to_lowercase()));
        assert!(error.description.contains("not supported in GridCraft (no disk/network/system access)"));
    }
    assert!(Program::parse("Synthetic", "Declare Sub UnsafeFeature Lib \"synthetic\"()").unwrap_err().description.contains("Declare"));
}

#[test]
fn allocation_limits_bypass_handlers_and_stop_halts() {
    for (body, limits) in [
        ("x = Space(100)", Limits { max_string_length: 10, ..Limits::default() }),
        ("Dim a(100) As Long", Limits { max_array_elements: 10, ..Limits::default() }),
        ("x = Space(100)", Limits { max_memory_bytes: 64, ..Limits::default() }),
    ] {
        let p = Program::parse("Synthetic", &format!("Sub Test()\nOn Error Resume Next\n{body}\nEnd Sub")).unwrap();
        let error = Interpreter::new(&p, NoHost, FixedClock, limits).run("Synthetic", "Test", &[]).unwrap_err();
        assert!(error.fatal);
        assert_eq!(error.number, -32000);
    }
    let p = Program::parse("Synthetic", "Sub Test()\nOn Error Resume Next\nStop\nEnd Sub").unwrap();
    let error = Interpreter::new(&p, NoHost, FixedClock, Limits::default()).run("Synthetic", "Test", &[]).unwrap_err();
    assert!(error.fatal);
    assert_eq!(error.number, -32001);
}

#[test]
fn implicit_byref_and_reference_expression_evaluated_once() {
    let source = "Private calls As Long\nFunction Index() As Long\ncalls = calls + 1\nIndex = 1\nEnd Function\nSub Bump(ByRef n)\nn = n + 1\nEnd Sub\nFunction Test()\nBump x\nDim values(1 To 1) As Long\nBump values(Index())\nTest = x + values(1) + calls\nEnd Function";
    assert_eq!(run(source).number().unwrap(), 3.0);
}

#[test]
fn typed_literals_array_inspection_and_option_base() {
    for (body, expected) in [
        ("Test = TypeName(1#)", Value::String("Double".into())),
        ("Test = TypeName(1&)", Value::String("Long".into())),
        ("Test = TypeName(40000)", Value::String("Long".into())),
        ("Dim a(2) As Long\nTest = TypeName(a)", Value::String("Long()".into())),
        ("Dim a(2) As Long\nTest = VarType(a)", Value::Long(8195)),
    ] {
        assert_eq!(run(&format!("Function Test()\n{body}\nEnd Function")), expected);
    }
    assert_eq!(run("Option Base 1\nFunction Test()\nTest = LBound(Array(1, 2))\nEnd Function").number().unwrap(), 1.0);
}

#[test]
fn text_compare_intrinsics_and_null_logic() {
    assert_eq!(
        run("Option Compare Text\nFunction Test()\nTest = StrComp(\"A\", \"a\") + InStr(\"ABC\", \"b\")\nEnd Function").number().unwrap(),
        2.0
    );
    for (expression, expected) in [
        ("False And Null", Value::Boolean(false)),
        ("True Or Null", Value::Boolean(true)),
        ("Null + 1", Value::Null),
        ("Empty + \"text\"", Value::String("text".into())),
    ] {
        assert_eq!(run(&format!("Function Test()\nTest = {expression}\nEnd Function")), expected);
    }
}

#[test]
fn parse_errors_include_source_location_and_nesting_is_bounded() {
    let error = Program::parse("Synthetic", "Sub Test()\nDim value As New Thing\nEnd Sub").unwrap_err();
    assert_eq!((&*error.module, &*error.procedure, error.line), ("Synthetic", "test", 2));
    let error = Program::parse("Synthetic", "Sub Test()\n\nx = \"unterminated\nEnd Sub").unwrap_err();
    assert_eq!(error.line, 3);
    let expression = format!("{}1{}", "(".repeat(100), ")".repeat(100));
    assert!(Program::parse("Synthetic", &format!("Function Test()\nTest = {expression}\nEnd Function")).unwrap_err().fatal);
}

#[test]
fn native_open_statement_is_denied_without_access() {
    let program = Program::parse("Synthetic", "Sub Test()\nOpen \"synthetic.txt\" For Input As #1\nEnd Sub").unwrap();
    let error = Interpreter::new(&program, NoHost, FixedClock, Limits::default()).run("Synthetic", "Test", &[]).unwrap_err();
    assert_eq!(error.number, 445);
    assert!(error.description.contains("Open not supported"));
}

#[test]
fn common_formats_text_split_and_ansi_conversions() {
    for (expression, expected) in [
        ("Format(7, \"000\")", "007"),
        ("Format(1.5, \"0.##\")", "1.5"),
        ("Format(-12.5, \"0.00;(0.00)\")", "(12.50)"),
        ("Format(1234.5, \"#,##0.00\")", "1,234.50"),
        ("Format(0.125, \"0.0%\")", "12.5%"),
        ("Format(#2024-01-02 13:45:06#, \"dd-mmm-yyyy hh:mm:ss AM/PM\")", "02-Jan-2024 01:45:06 PM"),
        ("Join(Split(\"aXbxc\", \"x\", -1, 1), \"-\")", "a-b-c"),
        ("Chr(128)", "€"),
        ("CStr(Asc(\"€\"))", "128"),
        ("CStr(InStrRev(\"abcabc\", \"bc\", -1))", "5"),
    ] {
        assert_eq!(run(&format!("Function Test()\nTest = {expression}\nEnd Function")), Value::String(expected.into()), "{expression}");
    }
}

#[test]
fn static_procedure_and_indexed_property() {
    let source = "Private saved(1 To 2) As Long\nProperty Get Item(ByVal index As Long) As Long\nItem = saved(index)\nEnd Property\nProperty Let Item(ByVal index As Long, ByVal amount As Long)\nsaved(index) = amount\nEnd Property\nStatic Function Count() As Long\nDim n As Long\nn = n + 1\nCount = n\nEnd Function\nFunction Test()\nItem(1) = Count()\nItem(2) = Count()\nTest = Item(1) + Item(2)\nEnd Function";
    assert_eq!(run(source).number().unwrap(), 3.0);
}

#[test]
fn arithmetic_preserves_types_and_detects_overflow() {
    for (expression, expected) in [
        ("TypeName(1 + 2)", "Integer"),
        ("TypeName(CLng(1) + 2)", "Long"),
        ("TypeName(CCur(1) + CCur(2))", "Currency"),
        ("TypeName(CSng(1) + 2)", "Single"),
        ("TypeName(1 And 2)", "Integer"),
    ] {
        assert_eq!(run(&format!("Function Test()\nTest = {expression}\nEnd Function")), Value::String(expected.into()), "{expression}");
    }
    assert_eq!(run("Function Test()\nOn Error Resume Next\nx = 32767 + 1\nTest = Err.Number\nEnd Function").number().unwrap(), 6.0);
    assert_eq!(run("Function Test()\nDim x As Variant\nx = 32767\nTest = x + 1\nEnd Function").number().unwrap(), 32768.0);
    assert_eq!(Value::LongLong(9_007_199_254_740_993).text().unwrap(), "9007199254740993");
}

#[test]
fn bounded_statement_nesting_and_log_accumulation() {
    let body = format!("{}Test = 1", "If True Then ".repeat(500));
    assert!(Program::parse("Synthetic", &format!("Function Test()\n{body}\nEnd Function")).unwrap_err().fatal);
    let program = Program::parse("Synthetic", "Sub Test()\nOn Error Resume Next\nx = Space(10)\nDebug.Print x, x\nEnd Sub").unwrap();
    let error = Interpreter::new(&program, NoHost, FixedClock, Limits { max_string_length: 15, ..Limits::default() })
        .run("Synthetic", "Test", &[])
        .unwrap_err();
    assert!(error.fatal);
    assert!(error.description.contains("log length"));
}

#[test]
fn project_extraction_bridge_and_qualified_calls() {
    let module = |name: &str, source: &str| gridcraft_vba::Module {
        name: name.into(),
        stream_name: name.into(),
        kind: gridcraft_vba::ModuleKind::Standard,
        source: source.into(),
        private_module: false,
        procedures: Vec::new(),
    };
    let project = gridcraft_vba::Project {
        name: "Synthetic".into(),
        code_page: 1252,
        modules: vec![
            module("Entry", "Attribute VB_Name = \"Entry\"\nFunction Test()\nTest = Helper.Amount()\nEnd Function"),
            module("Helper", "Function Amount()\nAmount = 42\nEnd Function"),
        ],
        references: Vec::new(),
        locked: false,
        warnings: Vec::new(),
    };
    let program = Program::from_project(&project).unwrap();
    assert_eq!(Interpreter::new(&program, NoHost, FixedClock, Limits::default()).run("Entry", "Test", &[]).unwrap().number().unwrap(), 42.0);
}

#[test]
fn public_private_cross_module_declarations_and_byref_calls() {
    let mut program = Program::parse(
        "Entry",
        "Option Explicit\nFunction Test()\nDim n As Long\nn = 3\nHelper.Bump n\nHelper.Total = n\nTest = Total + Helper.Total\nEnd Function",
    )
    .unwrap();
    program.modules.push(gridcraft_vbarun::Module::parse("Helper", "Public Total As Long\nPrivate hidden As Long\nPublic Sub Bump(ByRef n As Long)\nn = n + 1\nEnd Sub\nPrivate Sub HiddenProcedure()\nEnd Sub").unwrap());
    assert_eq!(Interpreter::new(&program, NoHost, FixedClock, Limits::default()).run("Entry", "Test", &[]).unwrap().number().unwrap(), 8.0);
    for expression in ["Helper.hidden", "Helper.HiddenProcedure()"] {
        program.modules[0] = gridcraft_vbarun::Module::parse("Entry", &format!("Function Test()\nTest = {expression}\nEnd Function")).unwrap();
        assert_eq!(Interpreter::new(&program, NoHost, FixedClock, Limits::default()).run("Entry", "Test", &[]).unwrap_err().number, 35);
    }
}

#[test]
fn comparison_empty_and_typed_string_with_numeric_variant() {
    for (setup, expression, expected) in [
        ("Dim x", "x = \"\"", true),
        ("Dim x", "\"\" = x", true),
        ("Dim x", "x < \"a\"", true),
        ("Dim x\nx = 7", "x <> \"\"", true),
        ("Dim x\nx = 7", "\"7\" = x", true),
        ("Dim x\nx = 7", "x < \"12\"", false),
        ("Dim x, y\nx = 7\ny = \"2\"", "x < y", true),
        ("Dim x, y\nx = 7\ny = \"2\"", "y > x", true),
        ("Dim x As Long, y\nx = 7\ny = \"7\"", "x = y", true),
    ] {
        let source = format!("Function Test()\n{setup}\nTest = {expression}\nEnd Function");
        assert_eq!(run(&source), Value::Boolean(expected), "{expression}");
    }
}
