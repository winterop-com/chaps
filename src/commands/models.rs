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
    /// Whether this project has the model enabled at all.
    enabled: bool,
    /// Host port in this project; `null` both for a model that is not enabled
    /// and for one that publishes none (`enabled` tells the two apart).
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
    /// Whether this ran inside a deployment directory at all, which is what
    /// tells "not enabled here" from "there is no here yet".
    in_project: bool,
    /// How to reach it from this machine, when it is enabled: its own host
    /// port, or chap-core's proxy for a service that publishes none.
    reach: Option<String>,
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
            // `--enabled` with nothing to show is a different fact from a
            // filter that matched nothing, and has an obvious next step.
            return if args.enabled {
                ctx.out
                    .backticks("no models enabled; run `chaps models enable ID` to add one")
            } else {
                "no models match".to_string()
            };
        }
        format!(
            "{}\n{}\n",
            table(&ctx.out, &rows, with_kind),
            ctx.out.backticks(&counted(&rows, project.is_some()))
        )
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
            return format!("no model matches `{}`", args.query);
        }
        // A search can turn up templates, so their kind is always shown.
        let closing = format!(
            "{} {} `{}`{}",
            rows.len(),
            if rows.len() == 1 {
                "model matches"
            } else {
                "models match"
            },
            args.query,
            enabled_clause(&rows, project.is_some())
        );
        format!(
            "{}\n{}\n",
            table(&ctx.out, &rows, true),
            ctx.out.backticks(&closing)
        )
    })
}

/// The line under a `models list` table: how many rows, and how many of them
/// this project runs.
fn counted(rows: &[ModelRow], in_project: bool) -> String {
    format!("{} listed{}", rows.len(), enabled_clause(rows, in_project))
}

/// `, 2 enabled in this project`, plus the way to enable one when none are.
///
/// Outside a deployment there is nothing to be enabled in, so the clause is
/// left off entirely rather than reported as zero.
fn enabled_clause(rows: &[ModelRow], in_project: bool) -> String {
    if !in_project {
        return String::new();
    }
    match rows.iter().filter(|r| r.enabled).count() {
        0 => ", none enabled in this project; enable one with `chaps models enable ID`".to_string(),
        count => format!(", {count} enabled in this project"),
    }
}

/// Show one model in full.
pub fn info(ctx: &Ctx, args: &ModelsInfoArgs) -> Result<()> {
    let project = open_project(ctx, false)?;
    let registry = registry::load(&ctx.registry)?;

    let model = registry
        .get(&args.id)
        .ok_or_else(|| ChapError::UnknownModel(args.id.clone()))?;
    let enabled = enabled_entry(project.as_ref(), model);
    let detail = ModelDetail {
        model,
        enabled,
        in_project: project.is_some(),
        reach: enabled
            .zip(project.as_ref())
            .map(|(e, p)| crate::status::reach(p, e.host_port, &e.service_id)),
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
    match ctx.project() {
        Ok(project) => Ok(Some(project)),
        Err(err) if !required && matches!(err.downcast_ref(), Some(ChapError::NotAProject(_))) => {
            Ok(None)
        }
        Err(err) => Err(err),
    }
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
        enabled: enabled.is_some(),
        enabled_port: enabled.and_then(|e| e.host_port),
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
                status_cell(out, r.assessed_status),
                r.stable.clone(),
                out.dim(&r.latest),
                enabled_cell(out, r),
            ];
            if with_kind {
                cells.push(out.dim(kind_label(r.kind)));
            }
            cells
        })
        .collect();
    out.table(&headers, &body)
}

/// The ENABLED cell: the host port when the model publishes one, `internal`
/// when it is enabled without one, and a dash when it is not enabled at all.
fn enabled_cell(out: &Out, row: &ModelRow) -> String {
    match (row.enabled, row.enabled_port) {
        (_, Some(port)) => out.key(&port.to_string()),
        (true, None) => out.dim("internal"),
        (false, None) => out.dim("-"),
    }
}

/// The STATUS cell, painted the colour the marketplace assessment means.
fn status_cell(out: &Out, status: AssessedStatus) -> String {
    let label = status_label(status);
    match status {
        AssessedStatus::Green => out.ok(label),
        // There is no orange in the 16-colour palette, and a half-verified
        // model is the same kind of "look before you run it" as a yellow one.
        AssessedStatus::Yellow | AssessedStatus::Orange => out.warn(label),
        AssessedStatus::Red => out.bad(label),
        AssessedStatus::Gray => out.dim(label),
    }
}

/// The STATUS cell of one version in `models info`.
fn version_status_cell(out: &Out, status: VersionStatus) -> String {
    let label = version_status_label(status);
    match status {
        VersionStatus::Verified => out.ok(label),
        VersionStatus::Unstable => out.warn(label),
        VersionStatus::Deprecated => out.warn(label),
        VersionStatus::Yanked => out.bad(label),
    }
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

    let fields = output::fields_with(
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
        &|label| out.dim(label),
    );

    let mut text = format!("{}\n\n{fields}", out.heading(&m.display_name));

    let versions: Vec<Vec<String>> = m
        .versions
        .iter()
        .map(|v| {
            vec![
                v.version.clone(),
                out.dim(&v.image_tag),
                version_status_cell(out, v.status),
                v.chapkit.clone(),
                out.dim(&first_line(v.changelog.as_deref())),
            ]
        })
        .collect();
    if !versions.is_empty() {
        text.push_str(&format!("\n{}\n", out.heading("versions")));
        text.push_str(&indent_block(
            &out.table(
                &["VERSION", "IMAGE TAG", "STATUS", "CHAPKIT", "CHANGELOG"],
                &versions,
            ),
            2,
        ));
    }

    if !m.configurations.is_empty() {
        text.push_str(&format!("\n{}\n", out.heading("configurations")));
        for (name, cfg) in &m.configurations {
            text.push_str(&format!("  {}\n", out.cmd(name)));
            if let Some(description) = &cfg.description {
                text.push_str(&indent_block(&output::wrapped(description, DETAIL_WRAP), 4));
            }
            let body = serde_json::to_string_pretty(&cfg.config)
                .unwrap_or_else(|_| cfg.config.to_string());
            text.push_str(&indent_block(&body, 4));
        }
    }

    if let Some(enabled) = detail.enabled {
        text.push_str(&format!("\n{}\n", out.heading("enabled in this project")));
        text.push_str(&output::fields_with(
            2,
            &[
                (
                    "reach",
                    detail
                        .reach
                        .clone()
                        .unwrap_or_else(|| match enabled.host_port {
                            Some(port) => port.to_string(),
                            None => "internal".to_string(),
                        }),
                ),
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
            &|label| out.dim(label),
        ));
    } else if detail.in_project {
        // The absence of the block above is easy to miss, and the next step
        // is the whole reason anyone reads this page.
        text.push_str(&format!(
            "\n{}\n",
            out.backticks(&format!(
                "not enabled in this project; enable it with `chaps models enable {}`",
                m.id
            ))
        ));
    } else {
        text.push_str(&format!(
            "\n{}\n",
            out.backticks(&format!(
                "not in a deployment directory; `chaps init` creates one, \
                 then `chaps models enable {}`",
                m.id
            ))
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
        enabled_on(Some(5001))
    }

    fn enabled_on(host_port: Option<u16>) -> EnabledModel {
        EnabledModel {
            service_id: "chapkit-ewars-model".into(),
            image: "ghcr.io/chap-models/chapkit_ewars_model".into(),
            image_tag: "sha-fa880a1".into(),
            version: "1.0.0".into(),
            channel: Some(Channel::Stable),
            host_port,
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
        assert!(!r.enabled);
        assert_eq!(enabled_cell(&Out::default(), &r), "-");
        assert!(
            r.image.starts_with(&format!("{}:", m.source.image)),
            "{} should be a tagged reference",
            r.image
        );
    }

    #[test]
    fn a_row_reports_the_port_of_an_enabled_model() {
        let r = row(&model("chapkit_ewars_model"), Some(&enabled()));
        assert!(r.enabled);
        assert_eq!(r.enabled_port, Some(5001));
        assert_eq!(enabled_cell(&Out::default(), &r), "5001");

        // Enabled without a port: `internal`, not a dash.
        let r = row(&model("chapkit_ewars_model"), Some(&enabled_on(None)));
        assert!(r.enabled);
        assert_eq!(r.enabled_port, None);
        assert_eq!(enabled_cell(&Out::default(), &r), "internal");
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
        let project = Project {
            dir: std::path::PathBuf::from("/tmp/chapx"),
            state: Default::default(),
        };
        let detail = ModelDetail {
            model: m,
            enabled,
            // Every `info` test below runs as if inside a deployment.
            in_project: true,
            reach: enabled.map(|e| crate::status::reach(&project, e.host_port, &e.service_id)),
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
        // Not enabled here, and the page says so on its last line rather than
        // leaving the missing block to be noticed.
        assert!(text.ends_with(
            "not enabled in this project; enable it with `chaps models enable chapkit_ewars_model`\n"
        ));
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
        assert!(text.contains("reach    http://localhost:5001"));
        assert!(text.contains("1.0.0 (stable)"));
        assert!(text.contains("compose.chapkit-ewars-model.yml"));
    }

    #[test]
    fn info_names_the_proxy_for_a_model_with_no_host_port() {
        let text = detail_of(&model("chapkit_ewars_model"), Some(&enabled_on(None)));
        assert!(
            text.contains(
                "reach    internal \
                 (proxy: http://localhost:8000/v2/services/chapkit-ewars-model/run/)"
            ),
            "{text}"
        );
    }

    #[test]
    fn info_json_adds_the_derived_fields_to_the_whole_model() {
        let m = model("chapkit_ewars_model");
        let detail = ModelDetail {
            model: &m,
            enabled: None,
            in_project: true,
            reach: None,
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
            "enabled",
            "enabled_port",
            "image",
            "requires_geo",
        ] {
            assert!(!first[key].is_null(), "{key} is missing or null");
        }
        assert_eq!(first["enabled_port"], 5001);
        assert_eq!(first["enabled"], true);

        // An enabled model with no host port is `enabled` with a null port,
        // which is what tells it apart from one that is not enabled at all.
        let rows = vec![row(&model("chapkit_ewars_model"), Some(&enabled_on(None)))];
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&rows).unwrap()).unwrap();
        assert_eq!(value[0]["enabled"], true);
        assert!(value[0]["enabled_port"].is_null());
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
