use mlua::Value;

/// Converts a Lua value to a Rust String (for debugging/display)
pub fn lua_value_to_string(value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.to_str().unwrap_or("").to_string(),
        Value::Table(_) => "table".to_string(),
        Value::Function(_) => "function".to_string(),
        Value::Thread(_) => "thread".to_string(),
        Value::UserData(_) => "userdata".to_string(),
        Value::LightUserData(_) => "lightuserdata".to_string(),
        Value::Error(e) => format!("error: {}", e),
    }
}
