//! `chaps models list|search|info` — read-only views of the catalogue.
//!
//! Owned by agent A. `enable` and `disable` live in
//! [`crate::commands::enable`] because they share the write path with `init`.

use crate::cli::{ModelsInfoArgs, ModelsListArgs, ModelsSearchArgs};
use crate::commands::Ctx;
use crate::error::{ChapError, Result};
use crate::output::{self, Out};
use crate::project::{EnabledModel, Project};
use crate::registry::{self, AssessedStatus, Channel, Kind, Model, VersionStatus};

/// One line of `models list` / `models search`, and one element of their
/// `--json` array.
#[derive(Debug, Clone, serde::Serialize)]
struct ModelRow {
    id: String,
    service_id: String,
    display_name: String,
    kind: Kind,
    assessed_status: AssessedStatus,
    /// Version the `stable` channel points at.
    stable: String,
    /// Version the `latest` channel points at.
    latest: String,
    /// Host port in this project, or `null` when the model is not enabled.
    enabled_port: Option<u16>,
    /// Deployable image reference for the stable channel.
    image: String,
    requires_geo: bool,
}

/// `models info`, and the shape of its `--json`: the whole marketplace entry
/// plus what the CLI derives from it.
#[derive(Debug, serde::Serialize)]
struct ModelDetail<'a> {
    #[serde(flatten)]
    model: &'a Model,
    /// This project's entry for the model, when it is enabled.
    enabled: Option<&'a EnabledModel>,
    image_stable: Option<String>,
    image_latest: Option<String>,
    needs_amd64: bool,
}

/// List marketplace models, filtered by the `--all`, `--templates` and
/// `--enabled` flags.
pub fn list(ctx: &Ctx, args: &ModelsListArgs) -> Result<()> {
    // The project is opened before the registry so `--enabled` outside a
    // project fails immediately instead of after a network fetch.
    let project = open_project(ctx, args.enabled)?;
    let registry = registry::load(&ctx.registry)?;

    let with_kind = args.all || args.templates;
    let rows: Vec<ModelRow> = registry
        .models
        .iter()
        .filter(|m| {
            if args.all {
                true
            } else if args.templates {
                m.is_template()
            } else {
                !m.is_template()
            }
        })
        .filter_map(|m| {
            let enabled = enabled_entry(project.as_ref(), m);
            match (args.enabled, enabled) {
                (true, None) => None,
                (_, enabled) => Some(row(m, enabled)),
            }
        })
        .collect();

    ctx.out.emit(&rows, || {
        if rows.is_empty() {
            "no models match".to_string()
        } else {
            table(&ctx.out, &rows, with_kind)
        }
    })
}

/// Search the catalogue by id, display name or summary.
pub fn search(ctx: &Ctx, args: &ModelsSearchArgs) -> Result<()> {
    let project = open_project(ctx, false)?;
    let registry = registry::load(&ctx.registry)?;

    let rows: Vec<ModelRow> = registry
        .search(&args.query)
        .into_iter()
        .map(|m| row(m, enabled_entry(project.as_ref(), m)))
        .collect();

    ctx.out.emit(&rows, || {
        if rows.is_empty() {
            format!("no model matches `{}`", args.query)
        } else {
            // A search can turn up templates, so their kind is always shown.
            table(&ctx.out, &rows, true)
        }
    })
}

/// Show one model in full.
pub fn info(ctx: &Ctx, args: &ModelsInfoArgs) -> Result<()> {
    let project = open_project(ctx, false)?;
    let registry = registry::load(&ctx.registry)?;

    let model = registry
        .get(&args.id)
        .ok_or_else(|| ChapError::UnknownModel(args.id.clone()))?;
    let detail = ModelDetail {
        model,
        enabled: enabled_entry(project.as_ref(), model),
        image_stable: channel_image(model, Channel::Stable),
        image_latest: channel_image(model, Channel::Latest),
        needs_amd64: model.needs_amd64(),
    };

    ctx.out.emit(&detail, || render_info(&ctx.out, &detail))
}

/// Load the project in `ctx.project_dir`, if there is one.
///
/// `required` is for `--enabled`, which is meaningless outside a project and
/// says so rather than silently listing nothing.
fn open_project(ctx: &Ctx, required: bool) -> Result<Option<Project>> {
    if Project::exists(&ctx.project_dir) {
        return Ok(Some(ctx.project()?));
    }
    if required {
        return Err(ChapError::NotAProject(ctx.project_dir.clone()).into());
    }
    Ok(None)
}

fn enabled_entry<'a>(project: Option<&'a Project>, model: &Model) -> Option<&'a EnabledModel> {
    project?.state.models.get(&model.id)
}

fn row(model: &Model, enabled: Option<&EnabledModel>) -> ModelRow {
    ModelRow {
        id: model.id.clone(),
        service_id: model.service_id.clone(),
        display_name: model.display_name.clone(),
        kind: model.kind,
        assessed_status: model.assessed_status,
        stable: model.channels.stable.clone(),
        latest: model.channels.latest.clone(),
        enabled_port: enabled.map(|e| e.host_port),
        // Falling back to the tagless reference keeps the column useful even
        // if a channel points at a version the file no longer lists.
        image: channel_image(model, Channel::Stable).unwrap_or_else(|| model.source.image.clone()),
        requires_geo: model.compatibility.requires_geo,
    }
}

fn table(out: &Out, rows: &[ModelRow], with_kind: bool) -> String {
    let mut headers = vec![
        "ID", "SERVICE", "NAME", "STATUS", "STABLE", "LATEST", "ENABLED",
    ];
    if with_kind {
        headers.push("KIND");
    }
    let body: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let mut cells = vec![
                r.id.clone(),
                r.service_id.clone(),
                r.display_name.clone(),
                status_label(r.assessed_status).to_string(),
                r.stable.clone(),
                r.latest.clone(),
                r.enabled_port
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ];
            if with_kind {
                cells.push(kind_label(r.kind).to_string());
            }
            cells
        })
        .collect();
    out.table(&headers, &body)
}

fn render_info(out: &Out, detail: &ModelDetail) -> String {
    let m = detail.model;

    let mut runtime = m.source.runtime_image.clone();
    if detail.needs_amd64 {
        // The R-INLA base has no arm64 build, so an Apple Silicon host runs it
        // under emulation; that is worth saying before someone waits for it.
        runtime.push_str("  (amd64 only)");
    }

    let mut attribution = m.attribution.author.clone();
    if let Some(org) = &m.attribution.organization {
        attribution.push_str(&format!(", {org}"));
    }
    if let Some(contact) = &m.attribution.contact {
        attribution.push_str(&format!(" <{contact}>"));
    }
    if let Some(citation) = &m.attribution.citation {
        attribution.push_str(&format!("\n{citation}"));
    }

    let mut covariates = Vec::new();
    if !m.covariates.required.is_empty() {
        covariates.push(format!("required: {}", m.covariates.required.join(", ")));
    }
    if !m.covariates.defaults.is_empty() {
        covariates.push(format!("defaults: {}", m.covariates.defaults.join(", ")));
    }
    covariates.push(format!(
        "extra covariates: {}",
        yes_no(m.covariates.allow_free_additional)
    ));

    let fields = output::fields(
        2,
        &[
            ("id", m.id.clone()),
            ("service", m.service_id.clone()),
            ("kind", kind_label(m.kind).to_string()),
            ("status", status_label(m.assessed_status).to_string()),
            ("summary", output::wrapped(&m.summary, DETAIL_WRAP)),
            ("repository", m.source.repository.clone()),
            (
                "image",
                channel_line(detail.image_stable.as_deref(), &m.channels.stable),
            ),
            (
                "latest",
                channel_line(detail.image_latest.as_deref(), &m.channels.latest),
            ),
            ("runtime", runtime),
            ("period types", m.compatibility.period_types.join(", ")),
            (
                "horizon",
                format!(
                    "{}-{} prediction periods",
                    m.compatibility.min_prediction_periods, m.compatibility.max_prediction_periods
                ),
            ),
            (
                "requires geo",
                yes_no(m.compatibility.requires_geo).to_string(),
            ),
            ("covariates", covariates.join("\n")),
            (
                "channels",
                format!("stable {}, latest {}", m.channels.stable, m.channels.latest),
            ),
            ("maintainers", m.maintainers.join(", ")),
            ("attribution", attribution),
        ],
    );

    let mut text = format!("{}\n\n{fields}", m.display_name);

    let versions: Vec<Vec<String>> = m
        .versions
        .iter()
        .map(|v| {
            vec![
                v.version.clone(),
                v.image_tag.clone(),
                version_status_label(v.status).to_string(),
                v.chapkit.clone(),
                first_line(v.changelog.as_deref()),
            ]
        })
        .collect();
    if !versions.is_empty() {
        text.push_str("\nversions\n");
        text.push_str(&indent_block(
            &out.table(
                &["VERSION", "IMAGE TAG", "STATUS", "CHAPKIT", "CHANGELOG"],
                &versions,
            ),
            2,
        ));
    }

    if !m.configurations.is_empty() {
        text.push_str("\nconfigurations\n");
        for (name, cfg) in &m.configurations {
            text.push_str(&format!("  {name}\n"));
            if let Some(description) = &cfg.description {
                text.push_str(&indent_block(&output::wrapped(description, DETAIL_WRAP), 4));
            }
            let body = serde_json::to_string_pretty(&cfg.config)
                .unwrap_or_else(|_| cfg.config.to_string());
            text.push_str(&indent_block(&body, 4));
        }
    }

    if let Some(enabled) = detail.enabled {
        text.push_str("\nenabled in this project\n");
        text.push_str(&output::fields(
            2,
            &[
                ("port", enabled.host_port.to_string()),
                (
                    "version",
                    match enabled.channel {
                        Some(channel) => {
                            format!("{} ({})", enabled.version, channel.as_str())
                        }
                        None => format!("{} (pinned)", enabled.version),
                    },
                ),
                ("image", format!("{}:{}", enabled.image, enabled.image_tag)),
                ("overlay", enabled.compose_file.clone()),
            ],
        ));
    }

    text
}

/// Width the prose in `models info` is wrapped to: the 80-column budget minus
/// the indent and label column [`output::fields`] adds.
const DETAIL_WRAP: usize = output::WRAP_WIDTH - 18;

/// `image:tag  (version)`, or just the version when the channel points at a
/// version the model file does not list.
fn channel_line(image: Option<&str>, version: &str) -> String {
    match image {
        Some(image) => format!("{image}  ({version})"),
        None => format!("{version} (no such version in this model file)"),
    }
}

fn channel_image(model: &Model, channel: Channel) -> Option<String> {
    let wanted = match channel {
        Channel::Stable => &model.channels.stable,
        Channel::Latest => &model.channels.latest,
    };
    model.version(wanted).map(|v| model.image_ref(v))
}

fn first_line(text: Option<&str>) -> String {
    text.unwrap_or("")
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn indent_block(text: &str, spaces: usize) -> String {
    let lead = " ".repeat(spaces);
    text.lines()
        .map(|line| {
            if line.is_empty() {
                String::from("\n")
            } else {
                format!("{lead}{line}\n")
            }
        })
        .collect()
}

fn status_label(status: AssessedStatus) -> &'static str {
    match status {
        AssessedStatus::Green => "green",
        AssessedStatus::Yellow => "yellow",
        AssessedStatus::Orange => "orange",
        AssessedStatus::Red => "red",
        AssessedStatus::Gray => "gray",
    }
}

fn version_status_label(status: VersionStatus) -> &'static str {
    match status {
        VersionStatus::Verified => "verified",
        VersionStatus::Unstable => "unstable",
        VersionStatus::Deprecated => "deprecated",
        VersionStatus::Yanked => "yanked",
    }
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Model => "model",
        Kind::Template => "template",
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> crate::registry::Registry {
        registry::load_embedded().expect("the embedded snapshot parses")
    }

    fn model(id: &str) -> Model {
        registry().get(id).expect("a vendored model").clone()
    }

    fn enabled() -> EnabledModel {
        EnabledModel {
            service_id: "chapkit-ewars-model".into(),
            image: "ghcr.io/chap-models/chapkit_ewars_model".into(),
            image_tag: "sha-fa880a1".into(),
            version: "1.0.0".into(),
            channel: Some(Channel::Stable),
            host_port: 5001,
            data_dir: "/app/data".into(),
            user: "chapkit:chapkit".into(),
            platform: Some("linux/amd64".into()),
            compose_file: "compose.chapkit-ewars-model.yml".into(),
        }
    }

    #[test]
    fn a_row_carries_the_channels_and_the_stable_image() {
        let m = model("chapkit_ewars_model");
        let r = row(&m, None);
        assert_eq!(r.id, "chapkit_ewars_model");
        assert_eq!(r.service_id, "chapkit-ewars-model");
        assert_eq!(r.kind, Kind::Model);
        assert_eq!(r.stable, m.channels.stable);
        assert_eq!(r.latest, m.channels.latest);
        assert_eq!(r.enabled_port, None);
        assert!(
            r.image.starts_with(&format!("{}:", m.source.image)),
            "{} should be a tagged reference",
            r.image
        );
    }

    #[test]
    fn a_row_reports_the_port_of_an_enabled_model() {
        let r = row(&model("chapkit_ewars_model"), Some(&enabled()));
        assert_eq!(r.enabled_port, Some(5001));
    }

    #[test]
    fn the_kind_column_is_optional_and_last() {
        let out = Out::default();
        let rows = vec![row(&model("chapkit_minimalist_example_py"), None)];

        let plain = table(&out, &rows, false);
        assert_eq!(
            plain.lines().next().unwrap().split_whitespace().last(),
            Some("ENABLED")
        );
        assert!(!plain.contains("template"));

        let with_kind = table(&out, &rows, true);
        assert_eq!(
            with_kind.lines().next().unwrap().split_whitespace().last(),
            Some("KIND")
        );
        assert!(with_kind.lines().nth(1).unwrap().ends_with("template"));
        assert!(with_kind.lines().all(|l| !l.ends_with(' ')));
    }

    #[test]
    fn an_unenabled_model_shows_a_dash() {
        let out = Out::default();
        let rows = vec![row(&model("auto_arima_chapkit"), None)];
        let text = table(&out, &rows, false);
        assert!(text.lines().nth(1).unwrap().ends_with(" -"), "{text}");
    }

    fn detail_of(m: &Model, enabled: Option<&EnabledModel>) -> String {
        let detail = ModelDetail {
            model: m,
            enabled,
            image_stable: channel_image(m, Channel::Stable),
            image_latest: channel_image(m, Channel::Latest),
            needs_amd64: m.needs_amd64(),
        };
        render_info(&Out::default(), &detail)
    }

    #[test]
    fn info_covers_every_documented_section() {
        let m = model("chapkit_ewars_model");
        let text = detail_of(&m, None);

        assert!(text.starts_with("CHAP-EWARS\n"));
        for needle in [
            "id            chapkit_ewars_model",
            "service       chapkit-ewars-model",
            "kind          model",
            "status        orange",
            "repository",
            "runtime",
            "period types",
            "horizon",
            "requires geo",
            "covariates",
            "channels",
            "maintainers",
            "attribution",
            "versions",
            "VERSION  IMAGE TAG",
            "configurations",
        ] {
            assert!(text.contains(needle), "{needle} missing from:\n{text}");
        }
        assert!(text.contains("(amd64 only)"), "ewars is an R-INLA model");
        assert!(text.contains(&m.channels.stable));
        assert!(!text.contains("enabled in this project"));
        assert!(text.lines().all(|l| !l.ends_with(' ')), "{text}");
    }

    #[test]
    fn info_wraps_the_summary_inside_eighty_columns() {
        // The versions table and image references can legitimately be wide;
        // prose must not be, so only the summary block is checked.
        for m in registry().models.iter() {
            let text = detail_of(m, None);
            let mut in_summary = false;
            for line in text.lines() {
                if line.starts_with("  summary ") {
                    in_summary = true;
                } else if in_summary && !line.starts_with("                  ") {
                    in_summary = false;
                }
                if in_summary {
                    assert!(
                        line.chars().count() <= output::WRAP_WIDTH,
                        "{}: {line}",
                        m.id
                    );
                }
            }
            assert!(text.contains("  summary "), "{} has no summary block", m.id);
        }
    }

    #[test]
    fn info_shows_the_project_entry_when_the_model_is_enabled() {
        let text = detail_of(&model("chapkit_ewars_model"), Some(&enabled()));
        assert!(text.contains("enabled in this project"));
        assert!(text.contains("port     5001"));
        assert!(text.contains("1.0.0 (stable)"));
        assert!(text.contains("compose.chapkit-ewars-model.yml"));
    }

    #[test]
    fn info_json_adds_the_derived_fields_to_the_whole_model() {
        let m = model("chapkit_ewars_model");
        let detail = ModelDetail {
            model: &m,
            enabled: None,
            image_stable: channel_image(&m, Channel::Stable),
            image_latest: channel_image(&m, Channel::Latest),
            needs_amd64: m.needs_amd64(),
        };
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&detail).unwrap()).unwrap();

        // Flattened, so the marketplace fields sit at the top level.
        assert_eq!(value["id"], "chapkit_ewars_model");
        assert_eq!(value["kind"], "model");
        assert_eq!(value["assessed_status"], "orange");
        assert!(value["versions"].is_array());
        assert!(value["configurations"].is_object());
        assert_eq!(value["needs_amd64"], true);
        assert_eq!(value["enabled"], serde_json::Value::Null);
        assert!(value["image_stable"].as_str().unwrap().contains(":sha-"));
        assert!(value["image_latest"].as_str().unwrap().contains(":sha-"));
    }

    #[test]
    fn row_json_has_the_documented_shape() {
        let rows = vec![row(&model("chapkit_ewars_model"), Some(&enabled()))];
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&rows).unwrap()).unwrap();
        let first = &value[0];
        for key in [
            "id",
            "service_id",
            "display_name",
            "kind",
            "assessed_status",
            "stable",
            "latest",
            "enabled_port",
            "image",
            "requires_geo",
        ] {
            assert!(!first[key].is_null(), "{key} is missing or null");
        }
        assert_eq!(first["enabled_port"], 5001);
    }

    #[test]
    fn a_dangling_channel_pointer_does_not_panic() {
        let mut m = model("auto_arima_chapkit");
        m.channels.latest = "9.9.9".to_string();
        assert!(channel_image(&m, Channel::Latest).is_none());
        let text = detail_of(&m, None);
        assert!(text.contains("no such version"), "{text}");
        // The row still has a usable image, from the stable channel.
        assert!(row(&m, None).image.contains(':'));
    }

    #[test]
    fn labels_match_the_yaml_spelling() {
        assert_eq!(status_label(AssessedStatus::Gray), "gray");
        assert_eq!(kind_label(Kind::Template), "template");
        assert_eq!(first_line(Some("first\nsecond")), "first");
        assert_eq!(first_line(None), "");
        assert_eq!(indent_block("a\nb", 2), "  a\n  b\n");
    }
}
