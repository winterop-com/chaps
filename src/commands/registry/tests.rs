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
