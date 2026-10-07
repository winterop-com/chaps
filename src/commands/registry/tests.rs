use super::*;
use crate::output::Out;

fn ctx(cache_dir: &std::path::Path) -> Ctx {
    Ctx {
        out: Out::default(),
        project_dir: std::path::PathBuf::from("."),
        registry: registry::RegistryOptions {
            url: "https://example.test/registry.yaml".to_string(),
            offline: true,
            cache_dir: cache_dir.to_path_buf(),
            timeout: registry::DEFAULT_TIMEOUT,
        },
        cli_version: "0.0.0-test",
    }
}

#[test]
fn the_report_names_the_cache_entry_for_this_url() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx(tmp.path());
    let registry = registry::load_embedded().unwrap();
    let report = report(&ctx, &registry, true);

    assert_eq!(report.models, registry.models.len());
    assert_eq!(report.ids.as_ref().unwrap().len(), registry.models.len());
    assert_eq!(report.ids.as_ref().unwrap()[0], "chapkit_ewars_model");
    assert!(matches!(report.provenance, Provenance::Embedded));
    assert!(
        report.cache_dir.starts_with(tmp.path().join("registries")),
        "{}",
        report.cache_dir.display()
    );
}

#[test]
fn update_only_reports_the_ids_on_show() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx(tmp.path());
    let registry = registry::load_embedded().unwrap();

    let value = serde_json::to_value(report(&ctx, &registry, false)).unwrap();
    assert!(value.get("ids").is_none(), "update omits the id list");
    assert_eq!(value["models"], registry.models.len());
    assert_eq!(value["provenance"]["kind"], "embedded");

    let value = serde_json::to_value(report(&ctx, &registry, true)).unwrap();
    assert_eq!(
        value["ids"].as_array().unwrap().len(),
        registry.models.len()
    );
}

#[test]
fn update_says_the_count_and_hints_where_it_came_from() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx(tmp.path());
    let registry = registry::load_embedded().unwrap();
    let report = report(&ctx, &registry, false);
    let mut lines = output::Report::default();
    updated(&report, &mut lines);
    let text = lines.text();
    assert!(
        text.starts_with(&format!(
            "updated the marketplace registry: {} models\n",
            registry.models.len()
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!("hint: fetched from {}\n", registry.url)),
        "{text}"
    );
}

#[test]
fn show_ends_on_the_catalogue_and_hints_the_next_command() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx(tmp.path());
    let registry = registry::load_embedded().unwrap();
    let mut report = report(&ctx, &registry, true);
    let mut lines = output::Report::default();
    shown(&report, &mut lines);
    assert_eq!(
        lines.text(),
        format!(
            "{} models in the catalogue (embedded)\n\
             hint: `varde models info ID` describes one\n",
            registry.models.len()
        )
    );

    report.models = 0;
    let mut lines = output::Report::default();
    shown(&report, &mut lines);
    assert!(
        lines.text().contains("run `varde registry update`"),
        "an empty catalogue names the way out at info level"
    );
}
