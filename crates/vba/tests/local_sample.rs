use std::io::{Cursor, Read, Write};

#[test]
#[ignore = "requires a private local XLS sample; prints names and levels only"]
fn local_xls_names_and_levels() {
    let path = std::env::var_os("GRIDCRAFT_VBA_SAMPLE").expect("set GRIDCRAFT_VBA_SAMPLE");
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.len() <= 128 * 1024 * 1024);
    let mut source = cfb::CompoundFile::open(Cursor::new(bytes)).unwrap();
    let root = std::path::Path::new("/_VBA_PROJECT_CUR");
    let entries: Vec<_> = source.walk_storage(root).unwrap().collect();
    assert!(entries.len() <= 16_384);
    let mut output = cfb::CompoundFile::create_with_version(cfb::Version::V3, Cursor::new(Vec::new())).unwrap();
    let mut total = 0usize;
    for entry in entries {
        let relative = entry.path().strip_prefix(root).unwrap();
        assert!(relative.components().count() <= 128);
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = std::path::Path::new("/").join(relative);
        if entry.is_storage() {
            output.create_storage_all(target).unwrap();
        } else if entry.is_stream() {
            let mut data = Vec::new();
            source.open_stream(entry.path()).unwrap().take(16 * 1024 * 1024 + 1).read_to_end(&mut data).unwrap();
            total += data.len();
            assert!(data.len() <= 16 * 1024 * 1024 && total <= 64 * 1024 * 1024);
            output.create_stream(target).unwrap().write_all(&data).unwrap();
        }
    }
    output.flush().unwrap();
    let project = gridcraft_vba::analyze(&output.into_inner().into_inner()).unwrap();
    println!("{} modules, {} procedures", project.modules.len(), project.modules.iter().map(|m| m.procedures.len()).sum::<usize>());
    println!("code page {}, locked {}, {} warnings", project.code_page, project.locked, project.warnings.len());
    for warning in &project.warnings {
        println!("warning: {warning}");
    }
    for module in project.modules {
        println!("module {}", module.name);
        for proc in module.procedures {
            println!("{}: level {}", proc.name, proc.compatibility.level);
        }
    }
}
