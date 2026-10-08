//! The questions of `varde models configs add -i`: the name, each user option
//! and the covariates. Each one shows the default, and Enter keeps it.

use crate::configs::{self, Config, Draft, Template, options};
use crate::error::Result;
use std::io::{BufRead, Write};

/// Ask in the terminal, with `start` as the defaults.
pub fn draft(template: &Template, existing: &[Config], model: &str, start: Draft) -> Result<Draft> {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut output = std::io::stderr();
    ask_all(&mut input, &mut output, template, existing, model, start)
}

/// [`draft`] on any input and output, which is what the tests give it.
pub fn ask_all(
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    template: &Template,
    existing: &[Config],
    model: &str,
    start: Draft,
) -> Result<Draft> {
    let names: Vec<&str> = existing
        .iter()
        .filter(|config| !config.archived)
        .map(|config| config.variant.as_str())
        .collect();
    writeln!(
        output,
        "configured model of {model}; press Enter to keep the value in brackets"
    )?;
    if !names.is_empty() {
        writeln!(
            output,
            "it has these configured models: {}",
            names.join(", ")
        )?;
    }
    let variant = ask(
        input,
        output,
        "name (the variant name the Modeling App shows)",
        &start.variant,
        |text| {
            configs::check_variant(text)?;
            match names.contains(&text) {
                true => Err(format!("{model} has a configured model {text} already")),
                false => Ok(text.to_string()),
            }
        },
    )?;

    let mut values = start.values.clone();
    for option in &template.options {
        if let Some(description) = &option.description {
            writeln!(output, "{}: {description}", option.key)?;
        }
        let shown = values
            .get(&option.key)
            .map(options::text_of)
            .unwrap_or_else(|| option.default_text());
        let label = format!("{} ({})", option.key, option.kind_name());
        let answer = ask(input, output, &label, &shown, |text| {
            match text.is_empty() {
                true => Ok(None),
                false => options::parse_value(option, text).map(Some),
            }
        })?;
        // A value equal to the default is left out, so the model keeps its
        // own default, as a marketplace configuration does.
        match answer {
            Some(value) if option.default.as_ref() != Some(&value) => {
                values.insert(option.key.clone(), value);
            }
            _ => {
                values.remove(&option.key);
            }
        }
    }

    let covariates = match template.free_covariates {
        true => {
            let required = match template.required_covariates.is_empty() {
                true => String::new(),
                false => format!(
                    "; every run gets {}",
                    template.required_covariates.join(", ")
                ),
            };
            let label = format!("additional covariates, separated by commas{required}");
            ask(input, output, &label, &start.covariates.join(","), |text| {
                Ok(configs::parse_covariates(text))
            })?
        }
        false => start.covariates.clone(),
    };
    Ok(Draft {
        variant,
        values,
        covariates,
    })
}

/// One question, asked again until `parse` takes the answer.
///
/// Enter alone answers with `shown`. The end of the input is a stop, not an
/// empty answer, so nothing is created from a closed terminal.
fn ask<T>(
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    label: &str,
    shown: &str,
    parse: impl Fn(&str) -> std::result::Result<T, String>,
) -> Result<T> {
    loop {
        write!(output, "{label} [{shown}]: ")?;
        output.flush()?;
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            writeln!(output)?;
            return Err(anyhow::anyhow!(
                "the input ended before the last question; nothing was created"
            ));
        }
        let text = match line.trim() {
            "" => shown,
            typed => typed,
        };
        match parse(text) {
            Ok(value) => return Ok(value),
            Err(why) => writeln!(output, "{why}")?,
        }
    }
}

#[cfg(test)]
mod tests;
