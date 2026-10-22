use std::collections::BTreeSet;
use std::io::Cursor;

use dxf::entities::EntityType;
use gridcraft_dxf::{Drawing, Units};

#[test]
fn r2000_structure_and_vietnamese_text() {
    let mut drawing = Drawing::new();
    drawing.units = Units::Millimeters;
    drawing.add_layer("Reactions", 1).unwrap();
    let text = drawing.add_text("Phản lực", [1.0, 2.0, 0.0], 250.0).unwrap();
    text.color = 3;
    text.layer = "Reactions".into();
    text.rotate([1.0, 2.0, 0.0], std::f64::consts::FRAC_PI_2).unwrap();
    drawing.add_point([3.0, 4.0, 5.0]).unwrap();
    drawing.add_lwpolyline(&[[0.0, 0.0], [10.0, 0.0], [10.0, 20.0]], true, 2.5).unwrap();
    drawing.add_lwpolyline(&[[0.0, 0.0], [1.0, 1.0]], false, 0.0).unwrap();
    drawing.add_line([0.0; 3], [5.0, 6.0, 7.0]).unwrap();
    let bytes = drawing.to_bytes().unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.is_ascii());
    assert!(text.contains("AC1015"));
    assert!(text.contains("\\U+1EA3"));
    assert!(text.ends_with("0\r\nEOF\r\n"));
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len() % 2, 0);
    let pairs: Vec<_> = lines.as_chunks::<2>().0.iter().map(|c| (c[0].parse::<u16>().unwrap(), c[1])).collect();
    let mut handles = BTreeSet::new();
    for (code, value) in &pairs {
        if *code == 5 {
            assert!(handles.insert(*value), "duplicate handle {value}");
        }
    }
    for (code, value) in &pairs {
        if matches!(code, 330 | 350 | 390) && *value != "0" {
            assert!(handles.contains(value), "dangling handle {value}");
        }
    }
    assert!(pairs.windows(2).any(|p| p == [(9, "$INSUNITS"), (70, "4")]));
    let parsed = dxf::Drawing::load(&mut Cursor::new(bytes)).unwrap();
    let entities: Vec<_> = parsed.entities().collect();
    assert_eq!(entities.len(), 5);
    match &entities[0].specific {
        EntityType::Text(t) => {
            assert_eq!(t.value, "Phản lực");
            assert_eq!(t.location.x, 1.0);
            assert_eq!(t.location.y, 2.0);
            assert_eq!(t.text_height, 250.0);
            assert!((t.rotation - 90.0).abs() < 1e-10);
        }
        other => panic!("unexpected entity: {other:?}"),
    }
    assert_eq!(entities[0].common.layer, "Reactions");
    assert_eq!(entities[0].common.color.index(), Some(3));
    assert_eq!(parsed.layers().find(|layer| layer.name == "Reactions").unwrap().color.index(), Some(1));
    match &entities[1].specific {
        EntityType::ModelPoint(p) => assert_eq!(p.location.z, 5.0),
        other => panic!("{other:?}"),
    }
    match &entities[2].specific {
        EntityType::LwPolyline(p) => {
            assert!(p.is_closed());
            assert_eq!(p.constant_width, 2.5);
            assert_eq!(p.vertices.len(), 3);
        }
        other => panic!("{other:?}"),
    }
    match &entities[3].specific {
        EntityType::LwPolyline(p) => assert!(!p.is_closed()),
        other => panic!("{other:?}"),
    }
    match &entities[4].specific {
        EntityType::Line(l) => assert_eq!(l.p2.z, 7.0),
        other => panic!("{other:?}"),
    }
}

#[test]
fn hostile_drawing_parameters_are_errors() {
    let mut drawing = Drawing::new();
    assert!(drawing.entity_mut(0).is_err());
    assert!(drawing.add_layer("bad\nname", 1).is_err());
    assert!(drawing.add_layer("ok", 256).is_err());
    assert!(drawing.add_text("text\n0\nEOF", [0.0; 3], 1.0).is_err());
    assert!(drawing.add_text("text", [0.0; 3], 0.0).is_err());
    assert!(drawing.add_point([f64::NAN, 0.0, 0.0]).is_err());
    assert!(drawing.add_lwpolyline(&[], false, 0.0).is_err());
    assert!(drawing.add_lwpolyline(&[[0.0; 2]; 2], false, -1.0).is_err());
    drawing.add_point([0.0; 3]).unwrap().layer = "missing".into();
    assert!(drawing.to_bytes().is_err());
}
