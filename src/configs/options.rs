//! The user options of a model template, and the values given for them.
//!
//! chap-core stores the options of a chapkit template as the `properties` of
//! the service's config schema (`_parse_user_options_from_config_schema` in
//! `models/external_chapkit_model.py`), without the reserved keys. That is a
//! JSON schema written by pydantic: a `type`, or an `anyOf` with `null` for an
//! optional field, an `enum` for a `Literal`, `items` for a list. chap-core
//! does not check the values of a chapkit configured model, and chapkit
//! accepts extra keys, so a mistake would only show at the first run. varde
//! checks them before the request.

use serde::Serialize;
use serde_json::{Map, Value as Json};

/// What values an option takes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Kind {
    Integer,
    Number,
    Boolean,
    String,
    /// A list of values of the inner kind.
    List {
        items: Box<Kind>,
    },
    /// One of these values.
    Enum {
        values: Vec<Json>,
    },
    /// A schema varde does not read, such as a reference or an object: any
    /// JSON value, or the text as a string.
    Any,
}

impl Kind {
    /// The name of the kind, as the messages and the form show it.
    pub fn name(&self) -> String {
        match self {
            Kind::Integer => "integer".to_string(),
            Kind::Number => "number".to_string(),
            Kind::Boolean => "true or false".to_string(),
            Kind::String => "text".to_string(),
            Kind::List { items } => format!("list of {}", items.plural()),
            Kind::Enum { values } => format!(
                "one of {}",
                values.iter().map(text_of).collect::<Vec<_>>().join(", ")
            ),
            Kind::Any => "JSON value".to_string(),
        }
    }

    /// The name with its article, for a sentence: "takes an integer".
    pub fn phrase(&self) -> String {
        match self {
            Kind::Integer => "an integer".to_string(),
            Kind::Number => "a number".to_string(),
            Kind::List { .. } | Kind::Any => format!("a {}", self.name()),
            other => other.name(),
        }
    }

    fn plural(&self) -> String {
        match self {
            Kind::Integer => "integers".to_string(),
            Kind::Number => "numbers".to_string(),
            Kind::Boolean => "true or false".to_string(),
            Kind::String => "text".to_string(),
            Kind::Any => "JSON values".to_string(),
            other => other.name(),
        }
    }

    /// An example value, for the messages.
    fn example(&self) -> String {
        match self {
            Kind::Integer => "3".to_string(),
            Kind::Number => "0.5".to_string(),
            Kind::Boolean => "true".to_string(),
            Kind::String => "abc".to_string(),
            Kind::List { items } => format!("{0},{0}", items.example()),
            Kind::Enum { values } => values.first().map(text_of).unwrap_or_default(),
            Kind::Any => "3".to_string(),
        }
    }
}

/// One user option of a template.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UserOption {
    pub key: String,
    #[serde(flatten)]
    pub kind: Kind,
    /// Whether `null` is a value it takes.
    pub nullable: bool,
    pub default: Option<Json>,
    pub description: Option<String>,
}

impl UserOption {
    /// The kind, with "or null" for an option that takes it.
    pub fn kind_name(&self) -> String {
        match self.nullable {
            true => format!("{} or null", self.kind.name()),
            false => self.kind.name(),
        }
    }

    /// The default as a value is typed, or `""` when there is none.
    pub fn default_text(&self) -> String {
        self.default.as_ref().map(text_of).unwrap_or_default()
    }
}

/// The options of a template's `userOptions`, sorted by key.
pub fn parse_options(user_options: &Json) -> Vec<UserOption> {
    let Some(properties) = user_options.as_object() else {
        return Vec::new();
    };
    let mut options: Vec<UserOption> = properties
        .iter()
        .filter(|(key, _)| !crate::configs::sync::RESERVED_CONFIG_KEYS.contains(&key.as_str()))
        .map(|(key, schema)| {
            let (kind, nullable) = kind_of(schema);
            UserOption {
                key: key.clone(),
                kind,
                nullable,
                default: schema.get("default").cloned(),
                description: schema
                    .get("description")
                    .or_else(|| schema.get("title"))
                    .and_then(Json::as_str)
                    .map(str::to_string),
            }
        })
        .collect();
    options.sort_by(|a, b| a.key.cmp(&b.key));
    options
}

/// The kind of one property, and whether it takes `null`.
fn kind_of(schema: &Json) -> (Kind, bool) {
    if let Some(values) = schema.get("enum").and_then(Json::as_array) {
        let nullable = values.iter().any(Json::is_null);
        let values = values.iter().filter(|v| !v.is_null()).cloned().collect();
        return (Kind::Enum { values }, nullable);
    }
    if let Some(value) = schema.get("const") {
        return (
            Kind::Enum {
                values: vec![value.clone()],
            },
            false,
        );
    }
    let branches = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .and_then(Json::as_array);
    if let Some(branches) = branches {
        let nullable = branches
            .iter()
            .any(|b| b.get("type").and_then(Json::as_str) == Some("null"));
        let rest: Vec<&Json> = branches
            .iter()
            .filter(|b| b.get("type").and_then(Json::as_str) != Some("null"))
            .collect();
        return match rest.as_slice() {
            [one] => (kind_of(one).0, nullable),
            _ => (Kind::Any, nullable),
        };
    }
    match schema.get("type") {
        Some(Json::String(name)) => (kind_named(name, schema), false),
        Some(Json::Array(names)) => {
            let nullable = names.iter().any(|n| n.as_str() == Some("null"));
            let rest: Vec<&str> = names
                .iter()
                .filter_map(Json::as_str)
                .filter(|n| *n != "null")
                .collect();
            match rest.as_slice() {
                [one] => (kind_named(one, schema), nullable),
                _ => (Kind::Any, nullable),
            }
        }
        _ => (Kind::Any, false),
    }
}

fn kind_named(name: &str, schema: &Json) -> Kind {
    match name {
        "integer" => Kind::Integer,
        "number" => Kind::Number,
        "boolean" => Kind::Boolean,
        "string" => Kind::String,
        "array" => Kind::List {
            items: Box::new(
                schema
                    .get("items")
                    .map(|items| kind_of(items).0)
                    .unwrap_or(Kind::Any),
            ),
        },
        _ => Kind::Any,
    }
}

/// A value as it is typed: a string bare, a list with commas, the rest as
/// JSON.
pub fn text_of(value: &Json) -> String {
    match value {
        Json::String(text) => text.clone(),
        Json::Array(items) if items.iter().all(|i| !i.is_array() && !i.is_object()) => {
            items.iter().map(text_of).collect::<Vec<_>>().join(",")
        }
        other => other.to_string(),
    }
}

/// `KEY=VALUE`, split at the first `=`.
pub fn parse_set(text: &str) -> Result<(String, String), String> {
    match text.split_once('=') {
        Some((key, value)) if !key.trim().is_empty() => {
            Ok((key.trim().to_string(), value.trim().to_string()))
        }
        _ => Err(format!(
            "`--set {text}` is not KEY=VALUE; write it as `--set n_lags=3`"
        )),
    }
}

/// The value `text` stands for as a value of `option`, or why it is not one.
pub fn parse_value(option: &UserOption, text: &str) -> Result<Json, String> {
    let text = text.trim();
    if option.nullable && text == "null" {
        return Ok(Json::Null);
    }
    parse_kind(&option.kind, text).ok_or_else(|| {
        format!(
            "`{text}` is not a value of `{}`, which takes {}{}, such as `{}`",
            option.key,
            option.kind.phrase(),
            if option.nullable { " or null" } else { "" },
            option.kind.example()
        )
    })
}

fn parse_kind(kind: &Kind, text: &str) -> Option<Json> {
    match kind {
        Kind::Integer => text.parse::<i64>().ok().map(Json::from),
        Kind::Number => text
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .and_then(|n| serde_json::Number::from_f64(n).map(Json::Number)),
        Kind::Boolean => match text.to_ascii_lowercase().as_str() {
            "true" => Some(Json::Bool(true)),
            "false" => Some(Json::Bool(false)),
            _ => None,
        },
        Kind::String => Some(Json::String(text.to_string())),
        Kind::Enum { values } => values
            .iter()
            .find(|value| {
                text_of(value) == text
                    || **value == serde_json::from_str::<Json>(text).unwrap_or(Json::Null)
            })
            .cloned(),
        Kind::List { items } => {
            // A JSON list is taken as it is, item by item; otherwise the
            // items are split at the commas.
            let parts: Vec<String> = match serde_json::from_str::<Json>(text) {
                Ok(Json::Array(values)) => values.iter().map(text_of).collect(),
                _ if text.is_empty() => Vec::new(),
                _ => text
                    .split(',')
                    .map(|part| part.trim().to_string())
                    .collect(),
            };
            parts
                .iter()
                .map(|part| parse_kind(items, part))
                .collect::<Option<Vec<Json>>>()
                .map(Json::Array)
        }
        Kind::Any => Some(
            serde_json::from_str::<Json>(text).unwrap_or_else(|_| Json::String(text.to_string())),
        ),
    }
}

/// The values of `sets` (each `(key, text)`), checked against `options`.
///
/// `model` is the model id the messages name. A key twice is a mistake: the
/// second one would hide the first.
pub fn values_of(
    options: &[UserOption],
    sets: &[(String, String)],
    model: &str,
) -> Result<Map<String, Json>, String> {
    let mut values = Map::new();
    for (key, text) in sets {
        let Some(option) = options.iter().find(|option| &option.key == key) else {
            return Err(unknown_key(options, key, model));
        };
        if values.contains_key(key) {
            return Err(format!("`{key}` is set twice; give each option once"));
        }
        values.insert(key.clone(), parse_value(option, text)?);
    }
    Ok(values)
}

/// The message for a key that is not an option of the template.
fn unknown_key(options: &[UserOption], key: &str, model: &str) -> String {
    if options.is_empty() {
        return format!(
            "`{key}` is not an option of {model}, which has no options; remove `--set`"
        );
    }
    format!(
        "`{key}` is not an option of {model}; its options are {}",
        options_list(options)
    )
}

/// `n_lags (integer), method (one of a, b)`.
pub fn options_list(options: &[UserOption]) -> String {
    options
        .iter()
        .map(|option| format!("{} ({})", option.key, option.kind_name()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The values in a short line, as the OPTIONS column shows them.
pub fn short(values: &Map<String, Json>, width: usize) -> String {
    let text = values
        .iter()
        .map(|(key, value)| format!("{key}={}", text_of(value)))
        .collect::<Vec<_>>()
        .join(" ");
    if text.chars().count() <= width {
        return text;
    }
    let cut: String = text.chars().take(width.saturating_sub(3)).collect();
    format!("{cut}...")
}

#[cfg(test)]
mod tests;
