use super::*;
use clap::Parser;

#[test]
fn from_cli_carries_the_global_flags() {
    let cli = Cli::try_parse_from([
        "chap",
        "--json",
        "-C",
        "/tmp/chapx",
        "--offline",
        "--cache-dir",
        "/tmp/chapcache",
        "--registry-url",
        "https://example.test/registry.yaml",
        "status",
    ])
    .unwrap();
    let ctx = Ctx::from_cli(&cli);
    assert!(ctx.out.json);
    assert_eq!(ctx.project_dir, PathBuf::from("/tmp/chapx"));
    assert!(ctx.registry.offline);
    assert_eq!(ctx.registry.cache_dir, PathBuf::from("/tmp/chapcache"));
    assert_eq!(ctx.registry.url, "https://example.test/registry.yaml");
    assert_eq!(ctx.cli_version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn cache_dir_falls_back_when_the_flag_is_absent() {
    let cli = Cli::try_parse_from(["chap", "status"]).unwrap();
    let ctx = Ctx::from_cli(&cli);
    assert_eq!(ctx.registry.cache_dir, crate::paths::cache_dir());
}
