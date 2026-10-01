use super::*;

const IMAGE: &str = "ghcr.io/chap-models/chapkit_ewars_model";
const TAG: &str = "sha-fa880a1";

fn request<'a>(id: &'a str, user: Option<&'a str>) -> Request<'a> {
    Request {
        id,
        image: IMAGE,
        image_tag: TAG,
        data_dir_flag: None,
        user_flag: user,
    }
}

/// Endpoints that reach nothing: the registry stub decides what is known.
fn endpoints() -> Endpoints {
    Endpoints {
        docker_probe: false,
        ..Endpoints::default()
    }
}

fn config(user: &str, working_dir: &str) -> ghcr::ImageConfig {
    ghcr::ImageConfig {
        user: user.to_string(),
        working_dir: working_dir.to_string(),
        amd64_only: false,
    }
}

/// [`from_image_with`] against a registry that answers with one config.
fn resolve(
    req: &Request,
    endpoints: &Endpoints,
    declared: Option<ghcr::ImageConfig>,
) -> Resolution {
    from_image_with(
        req,
        endpoints,
        &|_, _| Ok(declared.clone()),
        &|_| None,
        &|_| true,
        &|_, _| None,
    )
}

#[test]
fn an_image_that_runs_as_root_resolves_to_root() {
    for declared in ["root", "0", "0:0", ""] {
        let resolution = resolve(
            &request("chapkit_rwanda_malaria_bym_model", None),
            &endpoints(),
            Some(config(declared, "/work")),
        );
        assert_eq!(resolution.user, "root", "{declared:?}");
        assert_eq!(resolution.user_from, UserSource::ImageConfig);
        assert_eq!(resolution.data_dir, "/work/data");
        assert!(resolution.notes.is_empty(), "{:?}", resolution.notes);
    }
}

#[test]
fn an_account_name_resolves_to_the_numbers_the_chown_uses() {
    let resolution = resolve(
        &request("chapkit_ewars_model", None),
        &endpoints(),
        Some(config("chapkit", "/app")),
    );
    assert_eq!(resolution.user, "1000:1000");
    assert_eq!(resolution.user_from, UserSource::ImageConfig);
    assert_eq!(resolution.data_dir, "/app/data");
    assert!(resolution.notes.is_empty(), "{:?}", resolution.notes);
}

#[test]
fn a_name_nothing_knows_is_kept_with_a_note() {
    let resolution = resolve(
        &request("chapkit_ewars_model", None),
        &endpoints(),
        Some(config("app", "/app")),
    );
    assert_eq!(resolution.user, "app");
    assert_eq!(resolution.user_from, UserSource::ImageConfig);
    assert_eq!(resolution.notes.len(), 1, "{:?}", resolution.notes);
    assert!(
        resolution.notes[0].contains("--user"),
        "{:?}",
        resolution.notes
    );

    // Unless the image itself can be asked, which is the docker probe.
    let probed = from_image_with(
        &request("chapkit_ewars_model", None),
        &Endpoints::default(),
        &|_, _| Ok(Some(config("app", "/app"))),
        &|_| None,
        &|_| true,
        &|_, name| (name == "app").then_some((10001, 10001)),
    );
    assert_eq!(probed.user, "10001:10001");
    assert_eq!(probed.user_from, UserSource::DockerProbe);
}

#[test]
fn the_flag_wins_over_the_image() {
    let resolution = resolve(
        &request("chapkit_ewars_model", Some("1500:1600")),
        &endpoints(),
        Some(config("root", "/app")),
    );
    assert_eq!(resolution.user, "1500:1600");
    assert_eq!(resolution.user_from, UserSource::Flag);
    // The data directory still follows the image; only the user was asked
    // for on the command line.
    assert_eq!(resolution.data_dir, "/app/data");
}

/// A registry that will not answer falls through to the local image, and
/// only then to the table - with a note saying which one was used.
#[test]
fn the_local_image_comes_before_the_table_and_the_table_says_so() {
    let local = from_image_with(
        &request("chapkit_rwanda_malaria_bym_model", None),
        &Endpoints::default(),
        &|_, _| Err(anyhow::anyhow!("no route to host")),
        &|_| Some(("root".to_string(), "/work".to_string())),
        &|_| true,
        &|_, _| None,
    );
    assert_eq!(local.user, "root");
    assert_eq!(local.user_from, UserSource::DockerProbe);
    assert_eq!(local.data_dir, "/work/data");
    assert_eq!(local.notes.len(), 1, "the registry failure is worth saying");

    let table = from_image_with(
        &request("chapkit_ewars_model", None),
        &Endpoints {
            offline: true,
            ..Endpoints::default()
        },
        &|_, _| panic!("--offline asks no registry"),
        &|_| None,
        &|_| true,
        &|_, _| None,
    );
    assert_eq!(table.user, "1000:1000", "the table's `chapkit`, as numbers");
    assert_eq!(table.user_from, UserSource::Table);
    assert_eq!(table.data_dir, "/app/data");
    assert_eq!(table.notes.len(), 1);
    assert!(
        table.notes[0].contains("built-in table"),
        "{:?}",
        table.notes
    );
}

/// A local image store that cannot answer for the amd64 variant is not an
/// image that runs as root: [`crate::docker::image_config`] says "unknown"
/// for an empty config, and this is what that costs - the table, plus the
/// note that says nothing could be asked.
#[test]
fn a_local_image_store_that_cannot_answer_does_not_mean_root() {
    let resolution = from_image_with(
        &request("chapkit_ewars_model", None),
        &Endpoints::default(),
        &|_, _| Err(anyhow::anyhow!("no route to host")),
        &|_| None,
        &|_| true,
        &|_, _| None,
    );
    assert_eq!(
        resolution.user, "1000:1000",
        "the table's `chapkit`, not root"
    );
    assert_eq!(resolution.user_from, UserSource::Table);
    assert_eq!(resolution.data_dir, "/app/data");
    assert_eq!(resolution.notes.len(), 2, "{:?}", resolution.notes);
    assert!(
        resolution.notes[1].contains("nothing could say what"),
        "{:?}",
        resolution.notes
    );
}

#[test]
fn the_table_resolver_looks_nothing_up() {
    let resolution = from_table(&request("auto_arima_chapkit", None));
    assert_eq!(resolution.user, "root");
    assert_eq!(resolution.user_from, UserSource::Table);
    assert_eq!(resolution.data_dir, "/work/data");
    assert!(resolution.notes.is_empty());

    // A model the table has never heard of takes the chapkit defaults.
    let unknown = from_table(&request("something_else", None));
    assert_eq!(unknown.user, "1000:1000");
    assert_eq!(unknown.data_dir, overrides::DEFAULT_DATA_DIR);

    // And the flag still wins.
    let explicit = from_table(&request("chapkit_ewars_model", Some("0:0")));
    assert_eq!(explicit.user, "root", "0:0 is root, written the one way");
    assert_eq!(explicit.user_from, UserSource::Flag);
}

#[test]
fn the_repository_is_the_image_without_the_registry_host() {
    assert_eq!(
        request("x", None).repository(),
        "chap-models/chapkit_ewars_model"
    );
    assert_eq!(
        request("x", None).reference(),
        "ghcr.io/chap-models/chapkit_ewars_model:sha-fa880a1"
    );
}

#[test]
fn the_sources_read_as_labels() {
    assert_eq!(UserSource::Flag.label(), "--user");
    assert_eq!(UserSource::ImageConfig.label(), "image config");
    assert_eq!(UserSource::DockerProbe.label(), "docker probe");
    assert_eq!(UserSource::Table.label(), "table");
    assert_eq!(UserSource::default(), UserSource::Table);
}
