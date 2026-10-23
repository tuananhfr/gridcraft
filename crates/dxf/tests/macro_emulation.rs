use gridcraft_dxf::Drawing;
#[test]
fn macro_dxf_circle_arc_mtext_and_legacy_polyline_geometry() {
    let mut d = Drawing::new();
    d.add_circle([1.0, 2.0, 3.0], 4.0).unwrap();
    d.add_arc([0.0; 3], 2.0, 0.0, std::f64::consts::FRAC_PI_2).unwrap();
    d.add_polyline(&[[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]).unwrap();
    d.add_mtext("Synthetic\r\nTiếng Việt", [0.0; 3], 12.0).unwrap();
    let out = String::from_utf8(d.to_bytes().unwrap()).unwrap();
    for part in ["0\r\nCIRCLE\r\n", "0\r\nARC\r\n", "51\r\n90\r\n", "0\r\nPOLYLINE\r\n", "0\r\nVERTEX\r\n", "30\r\n0\r\n", "0\r\nMTEXT\r\n", "\\P"] {
        assert!(out.contains(part), "{part}");
    }
    assert_eq!(out.matches("0\r\nVERTEX\r\n").count(), 2);
    assert!(d.add_circle([0.0; 3], -1.0).is_err());
    assert!(d.add_arc([0.0; 3], 1.0, f64::NAN, 0.0).is_err());
    assert!(d.add_polyline(&[[f64::INFINITY; 3]; 2]).is_err());
}
