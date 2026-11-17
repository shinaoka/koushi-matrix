//! Command-line parsing driven by the MCP tool table: one command per tool,
//! one option per input-schema property.
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::{Failure, STDIN_ONLY_PROPERTIES};
use crate::tools::ToolDefinition;

const MAX_ARGV_BYTES: usize = 64 * 1024;
const MAX_ARGV_WORDS: usize = 64;

pub(super) enum Action {
    Help(String),
    Version,
    Call(Invocation),
}

pub(super) struct Invocation {
    pub tool: ToolDefinition,
    /// The argument object, or `None` until `--args-stdin` has been read.
    pub arguments: Option<Value>,
    pub endpoint: Option<PathBuf>,
    pub timeout: Duration,
    pub json: bool,
}

/// The option a schema property generates: `room_id` becomes `--room-id`.
pub(super) fn option_name(property: &str) -> String {
    format!("--{}", property.replace('_', "-"))
}

/// For an array of one-property objects (`attachments: [{path}]`), the name
/// of that property, which each repeated option value fills.
pub(super) fn item_property(property: &Value) -> Option<&str> {
    let items = &property["items"];
    if items["type"] != "object" {
        return None;
    }
    let properties = items["properties"].as_object()?;
    match properties.iter().next() {
        Some((name, item)) if properties.len() == 1 && item["type"] == "string" => {
            Some(name.as_str())
        }
        _ => None,
    }
}

pub(super) fn parse(args: impl Iterator<Item = OsString>) -> Result<Action, Failure> {
    let mut words = Vec::new();
    let mut size = 0;
    for arg in args {
        size += arg.len();
        if size > MAX_ARGV_BYTES || words.len() >= MAX_ARGV_WORDS {
            return Err(Failure::USAGE);
        }
        words.push(arg.into_string().map_err(|_| Failure::USAGE)?);
    }
    let leading_help = words.first().is_some_and(|word| word == "help");
    if leading_help || words.iter().any(|word| word == "--help" || word == "-h") {
        let named = words.get(usize::from(leading_help)).map(String::as_str);
        return Ok(Action::Help(match named.and_then(crate::tools::find) {
            Some(tool) => super::help::tool(&tool),
            None => super::help::overview(),
        }));
    }
    if words.len() == 1 && (words[0] == "--version" || words[0] == "-V") {
        return Ok(Action::Version);
    }
    let tool = words
        .first()
        .and_then(|name| crate::tools::find(name))
        .ok_or(Failure::USAGE)?;
    let mut invocation = Invocation {
        tool,
        arguments: None,
        endpoint: None,
        timeout: Duration::from_secs(75),
        json: false,
    };
    let properties = invocation.tool.input_schema["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut arguments = Map::new();
    let mut args_stdin = false;
    let mut seen = HashSet::new();
    let mut index = 1;
    while let Some(flag) = words.get(index) {
        index += 1;
        let property = flag
            .strip_prefix("--")
            .map(|name| name.replace('-', "_"))
            .filter(|name| option_name(name) == *flag)
            .filter(|name| !STDIN_ONLY_PROPERTIES.contains(&name.as_str()))
            .and_then(|name| properties.get(&name).map(|schema| (name, schema)));
        let repeatable = property
            .as_ref()
            .is_some_and(|(_, schema)| schema["type"] == "array");
        if !repeatable && !seen.insert(flag.as_str()) {
            return Err(Failure::USAGE);
        }
        match flag.as_str() {
            "--json" => {
                invocation.json = true;
                continue;
            }
            "--args-stdin" => {
                args_stdin = true;
                continue;
            }
            _ => {}
        }
        if let Some((name, schema)) = &property
            && schema["type"] == "boolean"
        {
            arguments.insert(name.clone(), json!(true));
            continue;
        }
        let value = words
            .get(index)
            .filter(|value| !value.trim().is_empty())
            .ok_or(Failure::USAGE)?;
        index += 1;
        match flag.as_str() {
            "--endpoint" => invocation.endpoint = Some(PathBuf::from(value)),
            "--timeout" => {
                let seconds = value
                    .parse::<u64>()
                    .ok()
                    .filter(|n| (1..=120).contains(n))
                    .ok_or(Failure::USAGE)?;
                invocation.timeout = Duration::from_secs(seconds);
            }
            _ => {
                let (name, schema) = property.ok_or(Failure::USAGE)?;
                let parsed = match schema["type"].as_str() {
                    Some("string") => json!(value),
                    Some("integer") => json!(value.parse::<i64>().map_err(|_| Failure::USAGE)?),
                    Some("array") => {
                        let element = match item_property(schema) {
                            Some(field) => json!({ field: value }),
                            None if schema["items"]["type"] == "string" => json!(value),
                            None => return Err(Failure::USAGE),
                        };
                        let list = arguments.entry(name).or_insert_with(|| json!([]));
                        list.as_array_mut().ok_or(Failure::INTERNAL)?.push(element);
                        continue;
                    }
                    _ => return Err(Failure::USAGE),
                };
                arguments.insert(name, parsed);
            }
        }
    }
    if args_stdin {
        if !arguments.is_empty() {
            return Err(Failure::USAGE);
        }
    } else {
        let arguments = Value::Object(arguments);
        check(&invocation.tool, &arguments)?;
        invocation.arguments = Some(arguments);
    }
    Ok(Action::Call(invocation))
}

/// The schema checks run before connecting, for options and stdin alike:
/// required properties and types (`validate_arguments`), then the bounds the
/// schema declares, which `validate_arguments` does not check.
pub(super) fn check(tool: &ToolDefinition, arguments: &Value) -> Result<(), Failure> {
    crate::tools::validate_arguments(tool.name, arguments).map_err(|_| Failure::USAGE)?;
    let properties = &tool.input_schema["properties"];
    for (name, value) in arguments.as_object().ok_or(Failure::USAGE)? {
        let schema = &properties[name];
        let within = |limit: &str, ok: fn(i64, i64) -> bool| match (
            schema[limit].as_i64(),
            value.as_i64(),
        ) {
            (Some(limit), Some(value)) => ok(value, limit),
            (Some(_), None) => !value.is_number(),
            (None, _) => true,
        };
        let long_enough = match (schema["minLength"].as_u64(), value.as_str()) {
            (Some(min), Some(text)) => text.chars().count() as u64 >= min,
            _ => true,
        };
        let few_enough = match (schema["maxItems"].as_u64(), value.as_array()) {
            (Some(max), Some(items)) => items.len() as u64 <= max,
            _ => true,
        };
        if !(within("minimum", |v, l| v >= l)
            && within("maximum", |v, l| v <= l)
            && long_enough
            && few_enough)
        {
            return Err(Failure::USAGE);
        }
    }
    Ok(())
}
