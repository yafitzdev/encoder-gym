use std::sync::OnceLock;

use serde::Serialize;
use serde_json::Value;

use crate::cli::{OutputFormat, PageArgs};

static OUTPUT: OnceLock<OutputFormat> = OnceLock::new();

pub fn set_output(output: OutputFormat) -> anyhow::Result<()> {
    OUTPUT
        .set(output)
        .map_err(|_| anyhow::anyhow!("CLI output mode was already initialized"))
}

pub fn print(value: &impl Serialize) -> anyhow::Result<()> {
    let value = serde_json::to_value(value)?;
    match OUTPUT.get().copied().unwrap_or(OutputFormat::Human) {
        OutputFormat::Json => println!("{}", serde_json::to_string(&value)?),
        OutputFormat::Human => print_human(&value)?,
    }
    Ok(())
}

pub fn print_page(items: &impl Serialize, returned: usize, page: PageArgs) -> anyhow::Result<()> {
    if page.summary {
        print(&serde_json::json!({
            "items": items,
            "page": {
                "limit": page.limit,
                "offset": page.offset,
                "returned": returned,
                "has_more": returned == page.limit as usize,
            }
        }))
    } else {
        print(items)
    }
}

fn print_human(value: &Value) -> anyhow::Result<()> {
    match value {
        Value::Array(items) if items.is_empty() => println!("No results."),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    println!();
                }
                print_human(item)?;
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                let label = key.replace('_', " ");
                match value {
                    Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                        println!("{label}: {}", scalar(value));
                    }
                    _ => {
                        println!("{label}:");
                        for line in serde_json::to_string_pretty(value)?.lines() {
                            println!("  {line}");
                        }
                    }
                }
            }
        }
        scalar_value => println!("{}", scalar(scalar_value)),
    }
    Ok(())
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "-".into(),
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}
