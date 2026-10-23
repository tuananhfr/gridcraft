mod support;
use gridcraft_vba::{ModuleKind, ProcKind, Scope, analyze, find_udf};
use support::{FixtureModule as M, compress, encrypt, fixture, record, unicode};

#[test]
fn spec_3_2_decompression_vectors_and_raw_chunks() {
    let vectors = [
        ("01 19 B0 00 61 62 63 64 65 66 67 68 00 69 6A 6B 6C 6D 6E 6F 70 00 71 72 73 74 75 76 2E", "abcdefghijklmnopqrstuv.".to_string()),
        (
            "01 2F B0 00 23 61 61 61 62 63 64 65 82 66 00 70 61 67 68 69 6A 01 38 08 61 6B 6C 00 20 6D 6E 6F 70 06 71 02 70 04 00 72 73 74 75 76 10 77 78 79 7A 00 2C",
            "#aaabcdefaaaaghijaaaaaklaaamnopqaaaaaaaaaaaarstuvwxyzaaa".to_string(),
        ),
        ("01 03 B0 02 61 45 00", "a".repeat(73)),
        ("01 09 B0 00 61 61 61 61 61 61 61 61 00", "a".repeat(8)),
    ];
    for (hex, expected) in vectors {
        let bytes = hex.split_whitespace().map(|v| u8::from_str_radix(v, 16).unwrap()).collect();
        let p = analyze(&fixture(&[M { name: "Vector", kind: "Module", source: bytes, private: false }], 1252, "", &[])).unwrap();
        assert_eq!(p.modules[0].source, expected);
    }
    let source = format!("{}\r\nSub Raw()\r\nEnd Sub\r\n", " ".repeat(8192));
    let p = analyze(&fixture(&[M::new("Raw", "Module", &source).unwrap()], 1252, "", &[])).unwrap();
    assert_eq!(p.modules[0].source, source);
    assert_eq!(p.modules[0].procedures[0].name, "Raw");
}

#[test]
fn kinds_headers_parameters_udfs_events_and_json() {
    let source = "Attribute VB_Name = \"Functions\"\r\nPublic Static Function SumIt(Optional ByVal text As String = \"it's,a\", _\r\nByRef count As Long, ParamArray rest() As Variant) As Double ' comment\r\nRem Shell CreateObject\r\ntext = \"Shell ' CreateObject\" ' Shell\r\nEnd Function\r\nPrivate Function Hidden()\r\nEnd Function\r\nSub Auto_Open()\r\nEnd Sub\r\nFriend Property Get Value() As String\r\nEnd Property\r\nProperty Let Value(ByVal rhs As String)\r\nEnd Property\r\nProperty Set Item(ByVal rhs As Object)\r\nEnd Property\r\n";
    let modules = [
        M::new("Functions", "Module", source).unwrap(),
        M::new("Book", "Document", "Sub Workbook_Open()\nEnd Sub\nSub Worksheet_Change(ByVal Target As Range)\nEnd Sub").unwrap(),
        M::new("Class1", "Class", "Function ClassFn()\nEnd Function").unwrap(),
        M::new("Form1", "BaseClass", "Private Sub CommandButton1_Click()\nEnd Sub").unwrap(),
        M::new("Internal", "Module", "Option Private Module\nFunction InternalFn()\nEnd Function").unwrap(),
    ];
    let p = analyze(&fixture(&modules, 1252, "", &[])).unwrap();
    assert_eq!(p.name, "Synthetic");
    assert_eq!(
        p.modules.iter().map(|m| m.kind).collect::<Vec<_>>(),
        [ModuleKind::Standard, ModuleKind::Document, ModuleKind::Class, ModuleKind::Form, ModuleKind::Standard]
    );
    let (module, proc) = find_udf(&p, "sUMiT").unwrap();
    assert_eq!(module.name, "Functions");
    assert_eq!(proc.kind, ProcKind::Function);
    assert_eq!(proc.line, 2);
    assert_eq!(proc.returns.as_deref(), Some("Double"));
    assert_eq!(proc.params.len(), 3);
    assert!(proc.params[0].by_val && proc.params[0].optional);
    assert_eq!(proc.params[0].default.as_deref(), Some("\"it's,a\""));
    assert_eq!(proc.params[1].type_name.as_deref(), Some("Long"));
    assert!(!proc.params[1].by_val);
    assert!(proc.params[2].param_array);
    assert_eq!(proc.compatibility.level, 1);
    assert!(proc.compatibility.reasons.is_empty());
    assert_eq!(p.modules[0].procedures[1].scope, Scope::Private);
    assert_eq!(p.modules[0].procedures[3].kind, ProcKind::PropertyGet);
    assert_eq!(p.modules[0].procedures[3].scope, Scope::Friend);
    assert_eq!(p.modules[0].procedures[4].kind, ProcKind::PropertyLet);
    assert_eq!(p.modules[0].procedures[5].kind, ProcKind::PropertySet);
    for name in ["Hidden", "Auto_Open", "ClassFn", "InternalFn"] {
        assert!(find_udf(&p, name).is_none());
    }
    assert!(p.modules[0].procedures[2].event.is_some());
    for proc in &p.modules[1].procedures {
        assert!(proc.event.is_some());
        assert_eq!(proc.compatibility.level, 1);
    }
    assert_eq!(p.modules[3].procedures[0].event.as_deref(), Some("CommandButton1_Click"));
    assert_eq!(p.modules[3].procedures[0].compatibility.level, 4);
    let json = serde_json::to_string(&p).unwrap();
    let roundtrip: gridcraft_vba::Project = serde_json::from_str(&json).unwrap();
    assert!(find_udf(&roundtrip, "SUMIT").is_some());
}

#[test]
fn compatibility_reasons_and_cycle_safe_calls() {
    let source = "Private Declare PtrSafe Function WinApi Lib \"x\" () As Long\nSub ExcelOnly()\nRange(\"A1\") = Application.WorksheetFunction.Sum(1, 2)\nMsgBox InputBox(Application.GetOpenFilename())\nEnd Sub\nSub OS()\nWinApi\nShell \"x\"\nSendKeys \"x\"\nx = Environ(\"PATH\")\nOpen \"x\" For Input As #1\nKill \"x\"\nFileCopy \"x\", \"y\"\nMkDir \"x\"\nx = Dir(\"x\")\nEnd Sub\nSub Com()\nx = CreateObject(\"Autocad.Application\")\nx = GetObject(\"x\")\nDim db As DAO.Database\nDim conn As ADODB.Connection\nx = OpenDatabase(\"x\")\nSet x = New ExternalClass\nx = ETABS.Application\nEnd Sub\nSub Forms()\nUserForm1.Show\nDim x As MSForms.Control\nEnd Sub\nSub A()\nB\nEnd Sub\nSub B()\nA\nCom\nEnd Sub\nSub Start()\nA\nEnd Sub";
    let p = analyze(&fixture(&[M::new("Macros", "Module", source).unwrap()], 1252, "", &[])).unwrap();
    let procs = &p.modules[0].procedures;
    assert_eq!(procs.iter().map(|p| p.compatibility.level).collect::<Vec<_>>(), [1, 2, 3, 4, 3, 3, 3]);
    let reasons = &procs[1].compatibility.reasons;
    for code in ["declare", "shell", "send_keys", "environment", "file_io"] {
        assert!(reasons.iter().any(|r| r.code == code));
    }
    assert!(reasons.iter().any(|r| r.code == "declare" && r.line == 7));
    for code in ["emulated_com", "get_object", "adodb", "new_object", "etabs"] {
        assert!(procs[2].compatibility.reasons.iter().any(|r| r.code == code));
    }
    assert!(procs[2].compatibility.reasons.iter().any(|r| r.code == "emulated_com" && r.detail == "CreateObject(\"autocad.application\")"));
    assert!(procs.iter().all(|p| p.compatibility.reasons.iter().all(|r| r.code != "calls" || !r.detail.contains(&format!("calls {} ", p.name)))));
    for proc in procs.iter().skip(4) {
        assert!(proc.compatibility.reasons.iter().any(|r| r.code == "calls" && r.detail.contains("level 3")));
    }
}

#[test]
fn protection_missing_source_and_code_pages() {
    let mut vietnamese = M::new("Vietnamese", "Module", "").unwrap();
    let text = "Function Tinh()\n' Tiếng Việt\nEnd Function";
    let (bytes, _, errors) = encoding_rs::WINDOWS_1258.encode(text);
    // Windows-1258 represents Vietnamese using combining tone marks.
    let text = if errors { "Function Tinh()\n' Viê\u{0323}t\nEnd Function" } else { text };
    let (bytes2, _, errors) = encoding_rs::WINDOWS_1258.encode(text);
    assert!(!errors);
    drop(bytes);
    vietnamese.source = compress(&bytes2).unwrap();
    let missing = M { name: "Removed", kind: "Module", source: vec![], private: true };
    let p = analyze(&fixture(&[vietnamese, missing], 1258, "DPB=\"1234\"\r\nCMG=\"1234\"\r\nGC=\"1234\"\r\n", &[])).unwrap();
    assert!(p.locked);
    assert_eq!(p.code_page, 1258);
    assert_eq!(p.modules[0].source, text);
    assert!(p.modules[1].source.is_empty());
    assert!(p.modules[1].private_module);
    assert!(p.warnings.iter().any(|w| w.contains("p-code only or source removed")));
    let text = "Function Wide()\n' λ\nEnd Function";
    let wide = M { name: "Wide", kind: "Module", source: compress(&unicode(text)).unwrap(), private: false };
    let p = analyze(&fixture(&[wide], 1200, "", &[])).unwrap();
    assert_eq!(p.modules[0].source, text);
}

#[test]
fn reference_record_layouts() {
    let mut refs = Vec::new();
    record(&mut refs, 0x16, b"Library");
    record(&mut refs, 0x3e, &unicode("Library"));
    let mut registered = Vec::new();
    registered.extend(3u32.to_le_bytes());
    registered.extend(b"reg");
    registered.extend([0; 6]);
    record(&mut refs, 0xd, &registered);
    let mut project = Vec::new();
    project.extend(3u32.to_le_bytes());
    project.extend(b"abs");
    project.extend(3u32.to_le_bytes());
    project.extend(b"rel");
    project.extend([0; 6]);
    record(&mut refs, 0xe, &project);
    record(&mut refs, 0x33, b"original");
    refs.extend(0x2fu16.to_le_bytes());
    refs.extend(0u32.to_le_bytes());
    refs.extend(4u32.to_le_bytes());
    refs.extend(b"twid");
    refs.extend([0; 6]);
    record(&mut refs, 0x16, b"Extended");
    record(&mut refs, 0x3e, &unicode("Extended"));
    refs.extend(0x30u16.to_le_bytes());
    refs.extend(0u32.to_le_bytes());
    refs.extend(3u32.to_le_bytes());
    refs.extend(b"ext");
    refs.extend([0; 26]);
    record(&mut refs, 0x33, b"standalone");
    let p = analyze(&fixture(&[], 1252, "", &refs)).unwrap();
    assert_eq!(p.references.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(), ["registered", "project", "control", "original"]);
    assert_eq!(p.references.iter().map(|r| r.libid.as_str()).collect::<Vec<_>>(), ["reg", "abs", "ext", "standalone"]);
    assert_eq!(p.references.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Library", "", "", ""]);
}

#[test]
fn protection_flags_are_decoded_without_touching_the_password() {
    let open = format!("DPB={}\r\nCMG={}\r\nGC={}\r\n", encrypt(0x17, &[0]), encrypt(0x2c, &[0; 4]), encrypt(0x41, &[0xff]));
    let p = analyze(&fixture(&[M::new("M", "Module", "Sub A()\nEnd Sub").unwrap()], 1252, &open, &[])).unwrap();
    assert!(!p.locked);
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    for protection in [
        format!("CMG={}\r\nGC={}\r\n", encrypt(0x06, &[1, 0, 0, 0]), encrypt(0x93, &[0xff])),
        format!("CMG={}\r\nGC={}\r\n", encrypt(0xa4, &[0; 4]), encrypt(0x02, &[0])),
    ] {
        let p = analyze(&fixture(&[M::new("M", "Module", "Sub A()\nEnd Sub").unwrap()], 1252, &protection, &[])).unwrap();
        assert!(p.locked);
        assert!(p.warnings.iter().any(|w| w.contains("not bypassed")));
        assert_eq!(p.modules[0].procedures.len(), 1);
    }
}

#[test]
fn malformed_procedures_warn_instead_of_failing() {
    let source = "Sub First()\nWorkbooks.Open Filename:=\"a.xls\", Format:=1: x = 1\nFunction Second()\nEnd Sub\nSub Third()\nMsgBox \"x\" _";
    let p = analyze(&fixture(&[M::new("M", "Module", source).unwrap()], 1252, "", &[])).unwrap();
    let procs = &p.modules[0].procedures;
    assert_eq!(procs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["First", "Second", "Third"]);
    assert_eq!(procs[0].compatibility.level, 1);
    for warning in ["First: missing procedure terminator", "Second: mismatched procedure terminator", "Third: missing procedure terminator"] {
        assert!(p.warnings.iter().any(|w| w.ends_with(warning)), "{warning}: {:?}", p.warnings);
    }
}

#[test]
fn every_truncation_and_2000_seeded_mutations_never_panic() {
    let bytes = fixture(&[M::new("Macros", "Module", "Function Test() As Long\nTest = 42\nEnd Function").unwrap()], 1252, "", &[]);
    for end in 0..bytes.len() {
        let _ = analyze(&bytes[..end]);
    }
    let mut seed = 0x4752494443524146u64;
    for _ in 0..2000 {
        let mut changed = bytes.clone();
        for _ in 0..4 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let i = seed as usize % changed.len();
            changed[i] ^= ((seed >> 32) as u8).max(1);
        }
        let _ = analyze(&changed);
    }
}

#[test]
fn malformed_tokens_chunks_and_limits_fail_cleanly() {
    for compressed in [vec![0], vec![1, 0, 0], vec![1, 1, 0xb0, 1, 0x61], vec![1, 2, 0xb0, 1, 0, 0], vec![1, 0, 0x30, 0]] {
        let p = analyze(&fixture(&[M { name: "Bad", kind: "Module", source: compressed, private: false }], 1252, "", &[])).unwrap();
        assert!(p.modules[0].source.is_empty());
        assert!(p.warnings.iter().any(|w| w.contains("source removed")));
    }
    assert!(analyze(&vec![0; 64 * 1024 * 1024 + 1]).is_err());
}

#[test]
fn continuations_keep_reason_lines_and_continue_comments() {
    let source = "Function Visible() As String\nRem ignored _\nShell \"x\"\nx = 1 ' note _\nKill \"x\"\nVisible = _\nCreateObject(\"External.Application\")\nEnd Function\n";
    let p = analyze(&fixture(&[M::new("M", "Module", source).unwrap()], 1252, "", &[])).unwrap();
    let proc = find_udf(&p, "Visible").unwrap().1;
    assert_eq!(proc.line, 1);
    assert!(proc.compatibility.reasons.iter().any(|r| r.code == "create_object" && r.line == 7));
    assert!(proc.compatibility.reasons.iter().all(|r| r.code != "shell" && r.code != "file_io"), "{:?}", proc.compatibility.reasons);
}

#[test]
fn decompression_and_line_budgets_are_enforced() {
    let mut stream = vec![1];
    for _ in 0..4097 {
        stream.extend([3, 0xb0, 2, b' ', 0xfc, 0x0f]);
    }
    let excessive = M { name: "Excessive", kind: "Module", source: stream, private: false };
    assert!(matches!(analyze(&fixture(&[excessive], 1252, "", &[])), Err(gridcraft_vba::Error::Limit(_))));
    let line = "x".repeat(256 * 1024 + 1);
    assert!(matches!(analyze(&fixture(&[M::new("Line", "Module", &line).unwrap()], 1252, "", &[])), Err(gridcraft_vba::Error::Limit(_))));
}

#[test]
fn total_decompression_and_metadata_budgets_are_enforced() {
    let mut stream = vec![1];
    for _ in 0..4096 {
        stream.extend([3, 0xb0, 2, b' ', 0xfc, 0x0f]);
    }
    let modules: Vec<_> =
        ["One", "Two", "Three", "Four"].iter().map(|name| M { name, kind: "Module", source: stream.clone(), private: false }).collect();
    assert!(matches!(analyze(&fixture(&modules, 1252, "", &[])), Err(gridcraft_vba::Error::Limit(_))));
    let mut refs = Vec::new();
    record(&mut refs, 0x16, &vec![b'x'; 1025]);
    assert!(matches!(analyze(&fixture(&[], 1252, "", &refs)), Err(gridcraft_vba::Error::Limit(_))));
    refs.clear();
    record(&mut refs, 0x7777, b"unknown layout");
    assert!(matches!(analyze(&fixture(&[], 1252, "", &refs)), Err(gridcraft_vba::Error::Invalid(_))));
}

#[test]
fn udf_lookup_is_case_insensitive_for_unicode_names() {
    let p = analyze(&fixture(&[M::new("Functions", "Module", "Function TÍNH()\nEnd Function").unwrap()], 65001, "", &[])).unwrap();
    assert!(find_udf(&p, "tính").is_some());
}

#[test]
fn macro_emulation_classifies_covered_factories_and_refuses_other_com() {
    let source = r#"
Sub Covered()
Dim db As DAO.Database
Set db = CreateObject("DAO.DBEngine.36").OpenDatabase("synthetic.mdb")
Set acad = GetObject(, "AutoCAD.Application")
End Sub
Sub Wrapper()
Covered
End Sub
Sub Unknown()
Set obj = CreateObject("Synthetic.Application")
End Sub
Sub Dynamic()
Set obj = CreateObject(progId)
End Sub
Sub RealEtabs()
Set obj = CreateObject("ETABSv1.Helper")
End Sub
Sub UncoveredGet()
Set obj = GetObject("drawing.dwg", "AutoCAD.Application")
End Sub
"#;
    let p = analyze(&fixture(&[M::new("Module1", "Module", source).unwrap()], 1252, "", &[])).unwrap();
    assert_eq!(p.modules[0].procedures.iter().map(|p| p.compatibility.level).collect::<Vec<_>>(), [1, 1, 3, 3, 3, 3]);
    assert!(p.modules[0].procedures[0].compatibility.reasons.iter().all(|r| r.code == "emulated_com"));
}
