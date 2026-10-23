//! Opt-in local customer acceptance; never prints workbook values or VBA source.
#![cfg(not(target_arch = "wasm32"))]
use gridcraft_engine::{Session, io, macro_host::MacroOptions};
use serde_json::{Value, json};

fn open(bytes: &[u8], name: &str) -> Session {
    let mut session = Session::new();
    session.execute("file.open", json!({"name":name,"base64":io::base64_encode(bytes)})).unwrap();
    session
}

fn report(name: &str, result: &Value, prompts: usize) {
    println!(
        "macro={name} status={} steps={} cellsChanged={} attemptedCellsChanged={} prompts={prompts}",
        result["status"].as_str().unwrap_or("failed"),
        result["steps"].as_u64().unwrap_or(0),
        result["cellsChanged"].as_u64().unwrap_or(0),
        result["attemptedCellsChanged"].as_u64().unwrap_or(0)
    );
    if let Some(error) = result.get("error").filter(|e| !e.is_null()) {
        println!(
            "error={} module={} line={}",
            error["message"].as_str().unwrap_or("unknown"),
            error["module"].as_str().unwrap_or(""),
            error["line"].as_u64().unwrap_or(0)
        );
    }
}

#[test]
#[ignore = "requires GRIDCRAFT_VBA_SAMPLE; optional GRIDCRAFT_VBA_MDB"]
fn customer_vba_acceptance() {
    let Some(path) = std::env::var_os("GRIDCRAFT_VBA_SAMPLE") else {
        println!("skipped: no GRIDCRAFT_VBA_SAMPLE");
        return;
    };
    let name = if std::path::Path::new(&path).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("xlsm")) {
        "acceptance.xlsm"
    } else {
        "acceptance.xls"
    };
    let bytes = std::fs::read(path).unwrap();
    let mdb = std::env::var_os("GRIDCRAFT_VBA_MDB").map(|path| std::fs::read(path).unwrap());
    let mut session = open(&bytes, name);
    let analysis = session.execute("macros.analyze", json!({})).unwrap();
    assert!(analysis["error"].is_null(), "VBA analysis failed");
    let procedures = analysis["procedures"].as_array().unwrap();
    println!(
        "modules={} procedures={} UDFs={}",
        analysis["modules"].as_array().unwrap().len(),
        procedures.len(),
        procedures.iter().filter(|p| p["udf"] == true).count()
    );
    for level in 1..=4 {
        println!("level={level} procedures={}", procedures.iter().filter(|p| p["compatibility"]["level"] == level).count());
    }
    session.execute("macros.consent", json!({"allow":true})).unwrap();
    session.execute("formulas.calculateNow", json!({})).unwrap();
    let usage = analysis["udfUsage"].as_array().unwrap();
    println!(
        "UDF cells recalculated={} runtimeErrors={}",
        usage.iter().filter(|u| u["udf"] == true).map(|u| u["count"].as_u64().unwrap_or(0)).sum::<u64>(),
        session.doc().unwrap().calc.external_errors.len()
    );
    for (name, needs_mdb) in [
        ("Module1.tinhthepmax", false),
        ("Module1.tinhthepall", false),
        ("Module1.PrintToPDF", false),
        ("Module1.InputD2", true),
        ("Sheet6.Input974", true),
        ("Sheet7.Input2017", true),
        ("Module1.InputData", true),
        ("Sheet6.Inputetabs", true),
        ("Sheet7.Inputetabs2017", true),
    ] {
        if needs_mdb && mdb.is_none() {
            println!("macro={name} skipped: no MDB");
            continue;
        }
        let mut session = open(&bytes, name);
        session.execute("macros.consent", json!({"allow":true})).unwrap();
        let mut task = match session.start_macro_interactive(name, MacroOptions::default()) {
            Ok(task) => task,
            Err(_) => {
                let log = session.execute("macros.log", json!({})).unwrap();
                if let Some(result) = log["runs"].as_array().and_then(|runs| runs.last()) {
                    report(name, result, 0);
                } else {
                    println!("macro={name} status=refused steps=0 cellsChanged=0 error=missing or ambiguous entry point");
                }
                continue;
            }
        };
        let mut prompts = 0;
        loop {
            if let Some(result) = session.step_macro(&mut task).unwrap() {
                report(name, &result, prompts);
                break;
            }
            if let Some((kind, _)) = task.take_prompt() {
                prompts += 1;
                let answer = match kind.as_str() {
                    "macro.msgBox" => json!(1),
                    "macro.inputBox" => json!(""),
                    "macro.filePicker" => json!(false),
                    "macro.openMdb" => mdb.as_ref().map(|b| json!({"base64":io::base64_encode(b)})).unwrap_or(Value::Null),
                    _ => {
                        task.cancel();
                        Value::Null
                    }
                };
                task.answer_prompt(answer);
            }
        }
    }
}
