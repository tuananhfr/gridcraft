use super::minimal;
use gridcraft_model::MacroButtonKind;

#[test]
fn macro_buttons_vml_ctrlprops_and_activex_are_read_once_with_assignments() {
    // Excel writes each control twice (x14 Choice and Fallback); the listing must hold it once.
    let sheet = r#"<sheetData/><legacyDrawing r:id="vml"/><controls xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:AlternateContent><mc:Choice Requires="x14"><control shapeId="1025" name="RunReport" r:id="props"><controlPr defaultSize="0"/></control></mc:Choice><mc:Fallback><control shapeId="1025" name="RunReport" r:id="props"/></mc:Fallback></mc:AlternateContent><control shapeId="1026" name="CommandButton1" r:id="active"/></controls>"#;
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="vml" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing" Target="../drawings/vmlDrawing1.vml"/><Relationship Id="props" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/ctrlProp" Target="../ctrlProps/ctrlProp1.xml"/><Relationship Id="active" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/control" Target="../activeX/activeX1.xml"/></Relationships>"#;
    let vml = r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:x="urn:schemas-microsoft-com:office:excel"><v:shape id="Button 1" o:spid="_x0000_s1025"><v:textbox><div><font>Run report</font></div></v:textbox><x:ClientData ObjectType="Button"><x:FmlaMacro>[0]!Module1.RunReport</x:FmlaMacro></x:ClientData></v:shape><v:shape id="_x0000_s1027"><v:textbox><div>Print</div></v:textbox><x:ClientData ObjectType="Button"><x:FmlaMacro>Module1.Print</x:FmlaMacro></x:ClientData></v:shape><v:shape id="note"><x:ClientData ObjectType="Note"/></v:shape></xml>"#;
    let props = r#"<formControlPr objectType="Button" fmlaMacro="[0]!Module1.RunReport"/>"#;
    let active = r#"<ax:ocx xmlns:ax="urn:activeX"><ax:ocxPr name="Caption" value="Refresh"/></ax:ocx>"#;
    let bytes = minimal(
        sheet,
        &[
            ("xl/worksheets/_rels/sheet1.xml.rels", rels),
            ("xl/drawings/vmlDrawing1.vml", vml),
            ("xl/ctrlProps/ctrlProp1.xml", props),
            ("xl/activeX/activeX1.xml", active),
        ],
        "",
        "",
    );
    let wb = crate::read_xlsx(&bytes).unwrap().0;
    assert_eq!(wb.macro_buttons.len(), 3);
    let report = wb.macro_buttons.iter().find(|b| b.name == "RunReport").unwrap();
    assert_eq!((report.sheet, report.caption.as_str(), report.macro_name.as_str()), (0, "Run report", "[0]!Module1.RunReport"));
    assert_eq!(report.kind, MacroButtonKind::FormControl);
    let print = wb.macro_buttons.iter().find(|b| b.caption == "Print").unwrap();
    assert_eq!(print.macro_name, "Module1.Print");
    let active = wb.macro_buttons.iter().find(|b| b.name == "CommandButton1").unwrap();
    assert_eq!(active.kind, MacroButtonKind::ActiveX);
    assert!(active.macro_name.is_empty());
    assert_eq!(active.caption, "Refresh");
}

#[test]
fn macro_buttons_ctrlprops_without_vml_are_read() {
    let bytes = minimal(
        r#"<sheetData/><controls><control shapeId="1" name="Go" r:id="p"/></controls>"#,
        &[
            (
                "xl/worksheets/_rels/sheet1.xml.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="p" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/ctrlProp" Target="../ctrlProps/ctrlProp1.xml"/></Relationships>"#,
            ),
            ("xl/ctrlProps/ctrlProp1.xml", r#"<formControlPr objectType="Button" fmlaMacro="Start"/>"#),
        ],
        "",
        "",
    );
    let wb = crate::read_xlsx(&bytes).unwrap().0;
    assert_eq!(wb.macro_buttons.len(), 1);
    assert_eq!(wb.macro_buttons[0].macro_name, "Start");
}
