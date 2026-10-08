//! The configured models of one model, as chap-core has them: a list, a form
//! that adds one, and a confirmation before anything is written.
//!
//! The reducer stays pure here too. What chap-core is asked, and when, is an
//! [`Effect`] that [`crate::tui::run_tui`] carries out; it hands the answer
//! back through [`App::configs_loaded`] and [`App::configs_done`].

use super::{Action, App, Effect, Mode, Outcome, Page};
use crate::configs::{self, Config, Draft, Template, options};

/// The footer note for `m` on a model this deployment does not run.
pub const CONFIGS_NEED_ENABLED: &str =
    "enable the model, save with s and start it with `varde up`; then press m";

/// The footer note for `m` in a deployment without chap-core.
pub const CONFIGS_NEED_CHAP_CORE: &str =
    "this deployment has no chap-core; enable it on the components page first";

/// What the view knows of chap-core.
#[derive(Debug, Clone, PartialEq)]
pub enum Load {
    /// The request is out.
    Loading,
    /// The configured models that are not archived, each with where it comes
    /// from, and the template when chap-core has one.
    Ready {
        template: Option<Template>,
        configs: Vec<(Config, &'static str)>,
    },
    /// Why chap-core could not tell, with the way out.
    Failed(String),
}

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
}

/// The form `a` opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub fields: Vec<Field>,
    pub cursor: usize,
    /// Why the last save was refused.
    pub error: Option<String>,
}

/// What a confirmation is about.
#[derive(Debug, Clone, PartialEq)]
pub enum Pending {
    Create(Draft),
    Archive { id: i64, variant: String },
}

/// The configured models page of one model.
#[derive(Debug, Clone, PartialEq)]
pub struct View {
    /// The marketplace id, and the service id chap-core knows it by.
    pub model: String,
    pub service: String,
    pub display_name: String,
    pub load: Load,
    pub cursor: usize,
    pub form: Option<Form>,
    pub pending: Option<Pending>,
}

impl View {
    /// The configured models, or none while there is no answer.
    pub fn configs(&self) -> &[(Config, &'static str)] {
        match &self.load {
            Load::Ready { configs, .. } => configs,
            _ => &[],
        }
    }

    fn template(&self) -> Option<&Template> {
        template_of(&self.load)
    }

    /// The effect that asks chap-core again.
    fn reload(&self) -> Effect {
        Effect::LoadConfigs {
            model: self.model.clone(),
            service: self.service.clone(),
        }
    }
}

fn template_of(load: &Load) -> Option<&Template> {
    match load {
        Load::Ready { template, .. } => template.as_ref(),
        _ => None,
    }
}

fn configs_of(load: &Load) -> Vec<Config> {
    match load {
        Load::Ready { configs, .. } => configs.iter().map(|(c, _)| c.clone()).collect(),
        _ => Vec::new(),
    }
}

/// The fields of the form for `template`: the name, each option, the
/// covariates when the template takes them.
pub fn form_for(template: &Template) -> Form {
    let mut fields = vec![Field {
        role: Role::Name,
        label: "name".to_string(),
        kind: "text".to_string(),
        description: "The variant name, which the Modeling App shows in brackets.".to_string(),
        default: String::new(),
        input: String::new(),
    }];
    for (at, option) in template.options.iter().enumerate() {
        fields.push(Field {
            role: Role::Option(at),
            label: option.key.clone(),
            kind: option.kind_name(),
            description: option.description.clone().unwrap_or_default(),
            default: option.default_text(),
            input: String::new(),
        });
    }
    if template.free_covariates {
        let description = match template.required_covariates.is_empty() {
            true => "Additional covariates, separated by commas.".to_string(),
            false => format!(
                "Additional covariates, separated by commas. Every run gets {}.",
                template.required_covariates.join(", ")
            ),
        };
        fields.push(Field {
            role: Role::Covariates,
            label: "covariates".to_string(),
            kind: "list of names".to_string(),
            description,
            default: String::new(),
            input: String::new(),
        });
    }
    Form {
        fields,
        cursor: 0,
        error: None,
    }
}

/// The configured model the form describes, or why it cannot be one.
///
/// An empty field is the default, and an option at its default is left out,
/// so the model keeps its own default.
pub fn draft_of(
    form: &Form,
    template: &Template,
    existing: &[Config],
    model: &str,
) -> Result<Draft, String> {
    let mut draft = Draft {
        variant: String::new(),
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
                if option.default.as_ref() != Some(&value) {
                    draft.values.insert(option.key.clone(), value);
                }
            }
        }
    }
    configs::check(&draft, template, existing, model)?;
    Ok(draft)
}

impl App<'_> {
    /// `m` on a model row: open its configured models and ask chap-core.
    pub(super) fn open_configs(&mut self) {
        if self.page != Page::Models {
            return;
        }
        let Some(row) = self.selected() else {
            return;
        };
        let model = self.model(row);
        if !self.initial_components.has_chap_core_api() {
            self.message = Some(CONFIGS_NEED_CHAP_CORE.to_string());
            return;
        }
        let Some(recorded) = self.initial.get(&model.id) else {
            self.message = Some(CONFIGS_NEED_ENABLED.to_string());
            return;
        };
        let view = View {
            model: model.id.clone(),
            service: recorded.service_id.clone(),
            display_name: model.display_name.clone(),
            load: Load::Loading,
            cursor: 0,
            form: None,
            pending: None,
        };
        self.effect = Some(view.reload());
        self.configs = Some(view);
        self.mode = Mode::Configs;
    }

    /// The answer to [`Effect::LoadConfigs`].
    pub fn configs_loaded(&mut self, load: Load) {
        if let Some(view) = &mut self.configs {
            view.load = load;
            view.cursor = view.cursor.min(view.configs().len().saturating_sub(1));
        }
    }

    /// The answer to a create or an archive: the footer line, and the list
    /// again.
    pub fn configs_done(&mut self, message: String, load: Load) {
        self.configs_loaded(load);
        self.message = Some(message);
    }

    /// The list: move, add, archive, ask again, or go back.
    pub(super) fn reduce_configs(&mut self, action: Action) -> Option<Outcome> {
        let Some(view) = &mut self.configs else {
            self.mode = Mode::Browse;
            return None;
        };
        let last = view.configs().len().saturating_sub(1);
        match action {
            Action::Down => view.cursor = (view.cursor + 1).min(last),
            Action::Up => view.cursor = view.cursor.saturating_sub(1),
            Action::Top => view.cursor = 0,
            Action::Bottom => view.cursor = last,
            Action::ConfigReload => {
                view.load = Load::Loading;
                self.effect = Some(view.reload());
            }
            Action::ConfigAdd => match (&view.load, view.template()) {
                (Load::Ready { .. }, Some(template)) => {
                    view.form = Some(form_for(template));
                    self.mode = Mode::ConfigForm;
                }
                (Load::Ready { .. }, None) => {
                    self.message = Some(format!(
                        "chap-core has no template of {} yet; when `varde status` shows it \
                         registered, press r",
                        view.model
                    ));
                }
                _ => {}
            },
            Action::ConfigArchive => {
                if let Some((config, _)) = view.configs().get(view.cursor) {
                    view.pending = Some(Pending::Archive {
                        id: config.id,
                        variant: config.variant.clone(),
                    });
                    self.mode = Mode::ConfigConfirm;
                }
            }
            Action::FilterCancel | Action::Quit => {
                self.configs = None;
                self.mode = Mode::Browse;
            }
            _ => {}
        }
        None
    }

    /// The form: type into a field, move between them, save or leave.
    pub(super) fn reduce_config_form(&mut self, action: Action) -> Option<Outcome> {
        let Some(view) = &mut self.configs else {
            self.mode = Mode::Browse;
            return None;
        };
        let Some(form) = &mut view.form else {
            self.mode = Mode::Configs;
            return None;
        };
        let last = form.fields.len().saturating_sub(1);
        match action {
            Action::FormChar(c) => {
                form.fields[form.cursor].input.push(c);
                form.error = None;
            }
            Action::FormBackspace => {
                form.fields[form.cursor].input.pop();
                form.error = None;
            }
            Action::Down => form.cursor = (form.cursor + 1).min(last),
            Action::Up => form.cursor = form.cursor.saturating_sub(1),
            Action::FormSubmit => {
                let existing = configs_of(&view.load);
                let template = template_of(&view.load)?;
                match draft_of(form, template, &existing, &view.model) {
                    Ok(draft) => {
                        view.pending = Some(Pending::Create(draft));
                        self.mode = Mode::ConfigConfirm;
                    }
                    Err(why) => form.error = Some(why),
                }
            }
            Action::FilterCancel | Action::Quit => {
                view.form = None;
                self.mode = Mode::Configs;
            }
            _ => {}
        }
        None
    }

    /// The confirmation: `y` writes, `n` goes back to where it came from.
    pub(super) fn reduce_config_confirm(&mut self, action: Action) -> Option<Outcome> {
        let Some(view) = &mut self.configs else {
            self.mode = Mode::Browse;
            return None;
        };
        let back = match view.form {
            Some(_) => Mode::ConfigForm,
            None => Mode::Configs,
        };
        match action {
            Action::ConfirmYes => {
                let effect = match view.pending.take() {
                    Some(Pending::Create(draft)) => Effect::CreateConfig {
                        model: view.model.clone(),
                        service: view.service.clone(),
                        draft,
                    },
                    Some(Pending::Archive { id, variant }) => Effect::ArchiveConfig {
                        model: view.model.clone(),
                        service: view.service.clone(),
                        id,
                        variant,
                    },
                    None => return None,
                };
                view.form = None;
                view.load = Load::Loading;
                self.effect = Some(effect);
                self.mode = Mode::Configs;
            }
            Action::ConfirmNo | Action::FilterCancel | Action::Quit => {
                view.pending = None;
                self.mode = back;
            }
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests;
