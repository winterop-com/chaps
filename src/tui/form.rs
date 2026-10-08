//! The form of one configured model: a field for the name, one for each
//! user option of the template, and one for the covariates.
//!
//! The same form is a page of `varde ui` and the whole screen of `varde
//! models configs add` and `update` at a terminal, so both look and behave
//! the same. It is pure state: [`Form::apply`] takes an [`Action`] and says
//! when the user saved or left.

use crate::configs::{self, Config, Draft, Template, options};
use crate::tui::app::Action;

/// What the form says when Esc leaves it.
pub const LEFT: &str = "left the form; nothing was changed";

/// What one field of the form is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Name,
    /// The user option at this index of the template's options.
    Option(usize),
    Covariates,
}

/// One field of the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub role: Role,
    pub label: String,
    /// The kind of value, such as `integer`.
    pub kind: String,
    pub description: String,
    /// What an empty field stands for.
    pub default: String,
    pub input: String,
    /// Whether the configured model had a value for it when the form opened.
    pub had: bool,
}

/// The form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub fields: Vec<Field>,
    pub cursor: usize,
    /// Why the last save was refused.
    pub error: Option<String>,
    /// The name of the configured model an update changes, which the form
    /// does not ask for.
    pub fixed_name: Option<String>,
}

/// What one action did to the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Nothing that ends it.
    Edited,
    /// Enter: check it and save it.
    Submit,
    /// Esc: leave it and write nothing.
    Cancel,
}

impl Form {
    /// Apply one key's action.
    pub fn apply(&mut self, action: Action) -> Event {
        let last = self.fields.len().saturating_sub(1);
        match action {
            Action::FormChar(c) => {
                if let Some(field) = self.fields.get_mut(self.cursor) {
                    field.input.push(c);
                }
                self.error = None;
            }
            Action::FormBackspace => {
                if let Some(field) = self.fields.get_mut(self.cursor) {
                    field.input.pop();
                }
                self.error = None;
            }
            Action::Down => self.cursor = (self.cursor + 1).min(last),
            Action::Up => self.cursor = self.cursor.saturating_sub(1),
            Action::FormSubmit => return Event::Submit,
            Action::FilterCancel | Action::Quit => return Event::Cancel,
            _ => {}
        }
        Event::Edited
    }
}

/// The fields of a new configured model of `template`: empty, so each one
/// shows the default it stands for.
pub fn form_for(template: &Template) -> Form {
    let mut fields = vec![Field {
        role: Role::Name,
        label: "name".to_string(),
        kind: "text".to_string(),
        description: "The variant name, which the Modeling App shows in brackets.".to_string(),
        default: String::new(),
        input: String::new(),
        had: false,
    }];
    for (at, option) in template.options.iter().enumerate() {
        fields.push(Field {
            role: Role::Option(at),
            label: option.key.clone(),
            kind: option.kind_name(),
            description: option.description.clone().unwrap_or_default(),
            default: option.default_text(),
            input: String::new(),
            had: false,
        });
    }
    if template.free_covariates || !template.allowed_covariates.is_empty() {
        let mut description = "Additional covariates, separated by commas.".to_string();
        if !template.free_covariates {
            description.push_str(&format!(
                " Only {} are allowed.",
                template.allowed_covariates.join(", ")
            ));
        }
        if !template.required_covariates.is_empty() {
            description.push_str(&format!(
                " Every run gets {}.",
                template.required_covariates.join(", ")
            ));
        }
        fields.push(Field {
            role: Role::Covariates,
            label: "covariates".to_string(),
            kind: "list of names".to_string(),
            description,
            default: String::new(),
            input: String::new(),
            had: false,
        });
    }
    Form {
        fields,
        cursor: 0,
        error: None,
        fixed_name: None,
    }
}

/// The fields of an update of `current`: no name, and each field holds the
/// value the configured model has now.
pub fn form_for_update(template: &Template, current: &Config) -> Form {
    let mut form = form_for(template);
    form.fields.retain(|field| field.role != Role::Name);
    for field in &mut form.fields {
        match field.role {
            Role::Option(at) => {
                if let Some(value) = current.values.get(&template.options[at].key) {
                    field.input = options::text_of(value);
                    field.had = true;
                }
            }
            Role::Covariates => field.input = current.covariates.join(","),
            Role::Name => {}
        }
    }
    form.fixed_name = Some(current.variant.clone());
    form
}

/// The configured model the form describes, or why it cannot be one.
///
/// An empty field is the default. A new value equal to the default is left
/// out, so the model keeps its own default; a value it had stays.
pub fn draft_of(
    form: &Form,
    template: &Template,
    existing: &[Config],
    model: &str,
) -> Result<Draft, String> {
    let mut draft = Draft {
        variant: form.fixed_name.clone().unwrap_or_default(),
        values: serde_json::Map::new(),
        covariates: Vec::new(),
    };
    for field in &form.fields {
        let text = field.input.trim();
        match field.role {
            Role::Name => draft.variant = text.to_string(),
            Role::Covariates => draft.covariates = configs::parse_covariates(text),
            Role::Option(at) => {
                let option = &template.options[at];
                if text.is_empty() {
                    continue;
                }
                let value = options::parse_value(option, text)?;
                if field.had || option.default.as_ref() != Some(&value) {
                    draft.values.insert(option.key.clone(), value);
                }
            }
        }
    }
    configs::check(&draft, template, existing, model)?;
    Ok(draft)
}

#[cfg(test)]
mod tests;
