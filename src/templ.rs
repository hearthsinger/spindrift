use std::collections::HashMap;
use tera::{Filter, Result, Value};

pub struct BatchFilter;

impl Filter for BatchFilter {
    fn filter(&self, value: &Value, args: &HashMap<String, Value>) -> Result<Value> {
        let items = match value {
            Value::Array(arr) => arr,
            _ => return Err(tera::Error::msg("`batch` only works on arrays!")),
        };

        let size = match args.get("size").and_then(|v| v.as_u64()) {
            Some(s) if s > 0 => s as usize,
            _ => return Err(tera::Error::msg("`batch` requires a `size` param > 0")),
        };

        let mut chunks = Vec::new();
        for chunk in items.chunks(size) {
            let curr = chunk.to_vec();
            chunks.push(Value::Array(curr));
        }

        Ok(Value::Array(chunks))
    }
}
