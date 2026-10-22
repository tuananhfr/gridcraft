mod support;

use gridcraft_mdb::{ColumnType, Database, Value};

const FIXTURES: &[&[u8]] = &[include_bytes!("fixtures/jet3.mdb"), include_bytes!("fixtures/jet4.mdb")];

#[test]
fn jet3_reads_concrete_values_and_types() {
    let db = Database::open(FIXTURES[0]).unwrap();
    let mut names: Vec<_> = db.tables().iter().map(|t| t.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["Table1", "Table2", "Table3", "Table4"]);
    let table = db.table("table1").unwrap();
    let types: Vec<_> = table.columns().iter().map(|c| c.kind).collect();
    assert_eq!(
        types,
        [
            ColumnType::Text,
            ColumnType::Text,
            ColumnType::Byte,
            ColumnType::Int,
            ColumnType::Long,
            ColumnType::Double,
            ColumnType::DateTime,
            ColumnType::Currency,
            ColumnType::Bool
        ]
    );
    let rows: Vec<_> = table.rows().collect::<Result<_, _>>().unwrap();
    let row = rows.iter().find(|r| r[0] == Value::Text("abcdefg".into())).unwrap();
    assert_eq!(
        &row[..6],
        &[
            Value::Text("abcdefg".into()),
            Value::Text("hijklmnop".into()),
            Value::Byte(2),
            Value::Int(222),
            Value::Long(333333333),
            Value::Double(444.555)
        ]
    );
    assert_eq!(row[8], Value::Bool(true));
    assert!(rows.iter().any(|r| r[0] == Value::Text("a".into()) && r[8] == Value::Bool(false)));
}

#[test]
fn jet4_follows_relocated_rows_without_duplicates() {
    let db = Database::open(FIXTURES[1]).unwrap();
    assert_eq!(db.tables().iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Table1"]);
    let table = db.table("Table1").unwrap();
    let rows: Vec<_> = table.rows().collect::<Result<_, _>>().unwrap();
    assert_eq!(rows.len(), 7);
    assert_eq!(
        rows[2],
        vec![
            Value::Null,
            Value::Text("row3col3".into()),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Text("row3col9".into()),
            Value::Null
        ]
    );
    assert_eq!(rows[4][1], Value::Text("row5col2".into()));
}

#[test]
fn etabs_names_scalars_nulls_and_all_memo_storage_modes() {
    for jet4 in [false, true] {
        let bytes = support::etabs(jet4);
        let db = Database::open(&bytes).unwrap();
        let mut names: Vec<_> = db.tables().iter().map(|t| t.name.as_str()).collect();
        names.sort();
        assert_eq!(names, [support::COORDINATES, support::CONTROL, support::SUPPORT]);
        let table = db.table(support::SUPPORT).unwrap();
        assert_eq!(table.columns()[9].kind, ColumnType::Currency);
        let rows: Vec<_> = table.rows().collect::<Result<_, _>>().unwrap();
        assert_eq!(rows.len(), 3, "deleted fourth row must be skipped");
        for row in rows {
            assert_eq!(
                row,
                vec![
                    Value::Text("J1".into()),
                    Value::Double(-123.5),
                    Value::Bool(true),
                    Value::DateTime(45000.5),
                    Value::Text(if jet4 { "Phản lực" } else { "Reaction" }.into()),
                    Value::Byte(255),
                    Value::Int(-222),
                    Value::Long(333333333),
                    Value::Single(1.25),
                    Value::Currency("-12.3456".into()),
                    Value::Numeric("-123.45".into()),
                    Value::Null
                ]
            );
        }
        let control = db.table(support::CONTROL).unwrap();
        assert_eq!(control.rows().next().unwrap().unwrap(), [Value::Text("tonf, m, C".into())]);
    }
}

fn consume(bytes: &[u8]) {
    if let Ok(db) = Database::open(bytes) {
        for t in db.tables() {
            if let Ok(table) = db.table(&t.name) {
                for row in table.rows() {
                    if row.is_err() {
                        break;
                    }
                }
            }
        }
    }
}

#[test]
fn truncated_and_bit_flipped_fixtures_never_panic() {
    for fixture in FIXTURES.iter().copied().chain([support::etabs(false), support::etabs(true)].iter().map(Vec::as_slice)) {
        for length in (0..fixture.len()).step_by((fixture.len() / 64).max(1)) {
            consume(&fixture[..length]);
        }
        for i in (0..fixture.len()).step_by((fixture.len() / 128).max(1)) {
            let mut damaged = fixture.to_vec();
            damaged[i] ^= 0xff;
            consume(&damaged);
        }
    }
}

#[test]
fn ace_and_cycles_are_readable_errors() {
    let mut bytes = support::etabs(true);
    bytes[20] = 2;
    assert!(Database::open(&bytes).err().unwrap().to_string().contains("Unsupported"));
    let mut bytes = support::etabs(true);
    bytes[2 * 4096 + 4..2 * 4096 + 8].copy_from_slice(&2u32.to_le_bytes());
    assert!(Database::open(&bytes).err().unwrap().to_string().contains("cyclic"));
}
