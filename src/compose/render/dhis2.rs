//! The `dhis2` component: its compose file, the seed-dump one-shot and the
//! scaffolded `dhis.conf`.

use super::{fill, normalize_newlines};
use crate::components::{DHIS2_CONFIG_FILE, DHIS2_DIR, DHIS2_TAG_ENV_VAR};
use crate::compose::spec::{Dhis2ConfigSpec, Dhis2SeedSource, Dhis2Spec};
use std::sync::LazyLock;

/// The `dhis2` component, as a compose file of its own.
pub(super) static DHIS2_TEMPLATE: LazyLock<String> =
    LazyLock::new(|| normalize_newlines(include_str!("../templates/compose.dhis2.yml")));
/// The `dhis.conf` a `dhis2` component is scaffolded with.
static DHIS2_CONFIG_TEMPLATE: LazyLock<String> =
    LazyLock::new(|| normalize_newlines(include_str!("../templates/dhis.conf")));

/// The one-shot that prepares the seed dump. A fragment rather than a file of
/// its own, like the model overlay's init container: a deployment that wants an
/// empty DHIS2 has no such service at all.
///
/// The two things it does that are not obvious are commented in place, because
/// both were learned from a restore that failed: published dumps are
/// `pg_dump --clean` without `--if-exists`, and a truncated download that gets
/// cached looks exactly like a dump with some of the data missing.
const DHIS2_DUMP_TEMPLATE: &str = r#"  dhis2-dump:
    # One-shot: fetches the seed dump, rewrites it so it loads into a fresh
    # database, and exits. dhis2-db mounts the same volume at
    # /docker-entrypoint-initdb.d/, so the restore is the stock postgres
    # entrypoint's - which runs it once, on a data directory it has just created,
    # and never again.
    image: @DUMP_IMAGE@
    restart: "no"
    working_dir: /opt/dump
    environment:
      DHIS2_DB_DUMP_URL: ${DHIS2_DB_DUMP_URL:-@SEED_URL@}
    volumes:
      - type: volume
        source: @DUMP_VOLUME@
        target: /opt/dump
@SEED_MOUNT@    entrypoint: ["/bin/sh", "-c"]
    command:
      - |
        set -eu
        if [ -f dump.sql.gz ]; then
          echo "dump.sql.gz is already prepared"
          exit 0
        fi
        rm -f raw.part out.part
        # A value that names a file is the one mounted above, which is copied in
        # place of the download; everything after this line is the same either
        # way, so an operator's own dump gets the same verification and the same
        # rewriting a published one does.
        if [ -f "$${DHIS2_DB_DUMP_URL}" ]; then
          echo "copying $${DHIS2_DB_DUMP_URL}"
          cp "$${DHIS2_DB_DUMP_URL}" raw.part
        else
          echo "downloading $${DHIS2_DB_DUMP_URL}"
          wget -O raw.part "$${DHIS2_DB_DUMP_URL}"
        fi
        # Never cache a truncated download: verify the gzip before transforming
        # it, verify the result, and publish with one atomic move. A half-finished
        # download that gets cached is a dump that restores without an error and
        # leaves most of the data out.
        gzip -t raw.part
        # Published dumps are `pg_dump --clean` WITHOUT `--if-exists`, so they
        # open on DROP and ALTER statements that assume the schema is already
        # there, and the postgres entrypoint runs init scripts under
        # `psql -v ON_ERROR_STOP=1`: the first missing object aborts the whole
        # restore. Retrofitting --if-exists makes that opening block a no-op on
        # an empty database. The substitutions are idempotent, so a dump that
        # already carries --if-exists is normalised rather than doubled, and each
        # one is anchored to the start of a line, so COPY data rows are left
        # alone. DROP EXTENSION and DROP SCHEMA go entirely: the postgis image
        # owns those objects, dropping them fails on the dependency, and the dump
        # recreates what it needs with CREATE EXTENSION IF NOT EXISTS.
        gunzip -c raw.part | sed -E '
          s/^ALTER TABLE (IF EXISTS )?(ONLY )?/ALTER TABLE IF EXISTS \2/
          s/^DROP (TABLE|INDEX|SEQUENCE|FUNCTION|VIEW|MATERIALIZED VIEW|TYPE|DOMAIN|AGGREGATE|TRIGGER) (IF EXISTS )?/DROP \1 IF EXISTS /
          /^DROP EXTENSION /d
          /^DROP SCHEMA /d
          s/^CREATE SCHEMA (IF NOT EXISTS )?/CREATE SCHEMA IF NOT EXISTS /
        ' | gzip > out.part
        gzip -t out.part
        mv out.part dump.sql.gz
        rm -f raw.part
        echo "wrote dump.sql.gz ($$(wc -c < dump.sql.gz) bytes)""#;

/// Container port DHIS2 listens on. Fixed, not a preference: the image's
/// `server.xml` declares one connector, on 8080, with the context path at the
/// root.
pub const DHIS2_CONTAINER_PORT: u16 = 8080;
/// Database image the component runs. PostGIS is not an optimisation here:
/// DHIS2's own startup check throws without the `postgis`, `pg_trgm` and
/// `btree_gin` extensions. This is the tag both `dhis2-core`'s own compose file
/// and the CHAP team's DHIS2 deployment use.
pub const DHIS2_DB_IMAGE: &str = "ghcr.io/baosystems/postgis:16-3.5";
/// Image the seed-dump one-shot runs: it needs `wget`, `gzip` and `sed -E` and
/// nothing else.
pub const DHIS2_DUMP_IMAGE: &str = "alpine:3.22";
/// Where a seed dump that is a file rather than a URL is mounted inside the
/// one-shot.
///
/// One fixed path, whatever the file is called on the host, so the script has a
/// single name to test for: `[ -f ]` on the value of `DHIS2_DB_DUMP_URL` is what
/// tells a mounted file from a URL to download, and no second variable decides
/// it.
pub const DHIS2_SEED_MOUNT: &str = "/opt/seed.sql.gz";
/// Named volume holding `/opt/dhis2`: the installed apps and the file store,
/// measured at about 429 MB on a real instance. Not optional - without it both
/// are lost on every recreate.
pub const DHIS2_HOME_VOLUME: &str = "dhis2_home";
/// Named volume holding the DHIS2 database's data directory.
pub const DHIS2_DB_VOLUME: &str = "dhis2_db";
/// Named volume the seed dump is prepared in, mounted into the database at
/// `/docker-entrypoint-initdb.d/`.
///
/// A download cache rather than data: the one-shot fetches the dump again when
/// this is empty. [`crate::components::Component::volumes`] still names it, so
/// the doctor does not call it a leftover and `--purge` takes it;
/// [`crate::backup::COMPONENT_VOLUMES`] leaves it out, so no archive carries it.
pub const DHIS2_DUMP_VOLUME: &str = "dhis2_dump";
/// The dump the `2.42` line is seeded from unless `.env` names another: the
/// DHIS2 climate demo, which is the database the Modeling App's own walkthrough
/// assumes.
///
/// The entry for that line in [`crate::components::DHIS2_SEED_DUMPS`], which is
/// where the other lines' dumps are and where a tag is looked up.
pub const DHIS2_DEFAULT_SEED_URL: &str =
    "https://databases.dhis2.org/climate/laos/2.42/laos.sql.gz";

/// Render `compose.dhis2.yml`: the `dhis2` component.
///
/// Four services and three volumes when the database is seeded from a dump, and
/// three of each without one. `dhis2` is the web service and everything else is
/// `dhis2-<something>`, so one glance at a container name says which component
/// owns it.
///
/// The seed is the only conditional part, and it is four blocks that travel
/// together: the one-shot that prepares the dump, the volume it writes into, the
/// database's mount of that volume at `/docker-entrypoint-initdb.d/`, and the
/// database's wait on the one-shot. Without a seed none of them is rendered, and
/// DHIS2 migrates an empty database on first boot instead.
///
/// A seed that is a file rather than a URL adds one bind mount to the one-shot
/// and changes nothing else: the mounted path is what `DHIS2_DB_DUMP_URL`
/// carries, and the script's `[ -f ]` on that value is what picks the copy over
/// the download. So both shapes go through the same verification and the same
/// rewriting, and `.env` can still move either of them.
///
/// The host port is published only when the component records one, as with
/// `ocs`: an instance behind a reverse proxy is `expose`d on the compose network
/// and nowhere else. There is no `platform:` line anywhere in the file - both
/// images are multi-arch, unlike the model images every overlay has to pin.
pub fn render_dhis2(spec: &Dhis2Spec) -> String {
    let mut port_lines = format!("    expose:\n      - \"{DHIS2_CONTAINER_PORT}\"\n");
    if let Some(port) = spec.host_port {
        port_lines.push_str(&format!(
            "    ports:\n      - \"{port}:{DHIS2_CONTAINER_PORT}\"\n"
        ));
    }
    if spec.host_gateway {
        port_lines.push_str(&format!(
            "    extra_hosts:\n      - \"{}:host-gateway\"\n",
            crate::components::HOST_GATEWAY
        ));
    }
    // Every seed block starts at the beginning of its line and carries its own
    // newlines, so a deployment without one leaves neither a comment nor a blank
    // line behind.
    let (dump_mount, dump_depends, dump_service, dump_volume_line) = match &spec.seed {
        None => (
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ),
        Some(seed) => (
            format!(
                "      # Where the postgres entrypoint looks for init scripts: the restore is\n\
                 \x20     # its own, on a data directory it has just created, and happens once.\n\
                 \x20     # Docker copies the image's postgis script into this volume as the\n\
                 \x20     # container is created, so that still runs, ahead of the dump.\n\
                 \x20     - type: volume\n\
                 \x20       source: {DHIS2_DUMP_VOLUME}\n\
                 \x20       target: /docker-entrypoint-initdb.d/\n"
            ),
            "    depends_on:\n      dhis2-dump:\n        condition: service_completed_successfully\n"
                .to_string(),
            format!(
                "\n{}\n",
                fill(
                    DHIS2_DUMP_TEMPLATE,
                    &[
                        ("DUMP_IMAGE", DHIS2_DUMP_IMAGE),
                        ("SEED_URL", seed_value(seed)),
                        ("SEED_MOUNT", &seed_mount_line(seed)),
                        ("DUMP_VOLUME", DHIS2_DUMP_VOLUME),
                    ],
                )
            ),
            format!("  {DHIS2_DUMP_VOLUME}: {{}}\n"),
        ),
    };
    fill(
        &DHIS2_TEMPLATE,
        &[
            ("IMAGE", &spec.image),
            ("TAG_VAR", DHIS2_TAG_ENV_VAR),
            ("IMAGE_TAG", &spec.image_tag),
            ("PORT_LINES", &port_lines),
            ("CONTAINER_PORT", &DHIS2_CONTAINER_PORT.to_string()),
            ("CONFIG_PATH", &format!("{DHIS2_DIR}/{DHIS2_CONFIG_FILE}")),
            ("DB_IMAGE", DHIS2_DB_IMAGE),
            ("HOME_VOLUME", DHIS2_HOME_VOLUME),
            ("DB_VOLUME", DHIS2_DB_VOLUME),
            ("DUMP_MOUNT", &dump_mount),
            ("DUMP_DEPENDS", &dump_depends),
            ("DUMP_SERVICE", &dump_service),
            ("DUMP_VOLUME_LINE", &dump_volume_line),
        ],
    )
}

/// What the one-shot's `DHIS2_DB_DUMP_URL` default carries: the URL to download,
/// or the path the mounted file has inside the container.
///
/// Public because `.env` writes the same value as the commented pin an operator
/// uncomments to move the seed. A mounted file's *host* path there would be a
/// path nothing inside the container can read, so both come from here.
pub fn seed_value(seed: &Dhis2SeedSource) -> &str {
    match seed {
        Dhis2SeedSource::Url(url) => url,
        Dhis2SeedSource::File(_) => DHIS2_SEED_MOUNT,
    }
}

/// The bind mount a file seed needs, and nothing at all for a URL.
///
/// The token sits at the start of its line and carries its own newline, so a URL
/// seed leaves no blank line in the volume list.
///
/// A relative path is spelled with a leading `./`: compose resolves it against
/// the project directory either way, but only that spelling is the one its own
/// documentation guarantees, and it is the spelling an operator recognises as a
/// path rather than a named volume.
fn seed_mount_line(seed: &Dhis2SeedSource) -> String {
    match seed {
        Dhis2SeedSource::Url(_) => String::new(),
        Dhis2SeedSource::File(path) => {
            let path = path.replace('\\', "/");
            // Judged on the string, not by the host: the compose file reads the
            // same wherever it was rendered, and `Path::is_absolute` on Windows
            // calls `/srv/dump.sql.gz` relative and would make it `.//srv/...`.
            let drive = path.as_bytes();
            let absolute = path.starts_with('/')
                || (drive.len() > 2 && drive[0].is_ascii_alphabetic() && &drive[1..3] == b":/");
            let source = if absolute || path.starts_with('.') {
                path
            } else {
                format!("./{path}")
            };
            format!("      - {source}:{DHIS2_SEED_MOUNT}:ro\n")
        }
    }
}

/// Render the scaffolded `dhis2/dhis.conf`.
///
/// Written once, when the component is enabled, and never again: it is the
/// operator's file from that moment on, exactly like `ocs/climate-service.yaml`.
///
/// The file is mandatory - DHIS2 throws on startup without it, and there is no
/// environment-only mode - and what the environment does is substitute `${...}`
/// placeholders into it, so the placeholders are written literally and the
/// compose file passes the matching variables. `fill` only ever replaces `@KEY@`,
/// which is why the two notations can share one file.
///
/// The only value filled here is the route allowlist, because it is the only one
/// a caller has to decide: DHIS2 v42 and later default it to `https://*` and then
/// refuse the `http://` target the Modeling App needs, since it reaches chap-core
/// through a DHIS2 Route rather than directly.
pub fn render_dhis2_config(spec: &Dhis2ConfigSpec) -> String {
    fill(
        &DHIS2_CONFIG_TEMPLATE,
        &[("ROUTE_ALLOWED", &spec.route_allowed)],
    )
}
