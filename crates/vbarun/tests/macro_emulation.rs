use gridcraft_vbarun::{Argument, FixedClock, Host, Interpreter, Limits, Object, Program, Result, Value};
struct Fields;
impl Host for Fields {
    fn global(&mut self, name: &str) -> Option<Value> {
        (name == "rs").then_some(Value::Object(Some(1)))
    }
    fn get_member(&mut self, _obj: Object, name: &str, args: &[Argument]) -> Result<Value> {
        assert_eq!(name, "");
        assert_eq!(args[0].value, Value::String("synthetic".into()));
        Ok(Value::Long(42))
    }
}
#[test]
fn macro_bang_field_selector_preserves_single_type_suffixes() {
    let p = Program::parse("Module1", "Function Main()\nDim n!\nn! = 1!\nMain = rs!Synthetic + n!\nEnd Function").unwrap();
    let result = Interpreter::new(&p, Fields, FixedClock, Limits::default()).run("Module1", "Main", &[]).unwrap();
    assert_eq!(result.number().unwrap(), 43.0);
}
