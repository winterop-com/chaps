//! Model overlays: one `compose.<service_id>.yml` per enabled model.

use super::labels::{ROLE_MODEL, labels_block};
use super::{fill, normalize_newlines};
use crate::compose::overrides;
use crate::compose::spec::OverlaySpec;
use std::sync::LazyLock;

/// One model service, as a compose overlay.
pub(super) static OVERLAY_TEMPLATE: LazyLock<String> =
    LazyLock::new(|| normalize_newlines(include_str!("../templates/compose.overlay.yml")));
/// The one-shot container that hands a model's data volume over before it
/// starts. A fragment rather than a file of its own, substituted into the
/// overlay: every model gets one, root included, where the chown is to `0:0`.
static OVERLAY_INIT_TEMPLATE: LazyLock<String> =
    LazyLock::new(|| normalize_newlines(include_str!("../templates/compose.overlay-init.yml")));

/// Render one `compose.<service_id>.yml` overlay.
///
/// An image that runs as root gets no `user:` line: there is nothing to
/// override, and root can write a fresh volume as docker seeded it. Every
/// image gets an init container, carrying the same numeric `uid:gid` as the
/// `user:` line where there is one - so compose and the `chown` cannot drift
/// apart, and the numbers because the init container is busybox, which
/// resolves no account name of its own.
pub fn render_overlay(spec: &OverlaySpec) -> String {
    // A local image has no registry behind it, so `compose pull` (which
    // `varde update` runs) has to skip it rather than fail.
    let pull_line = if crate::compose::is_local_image(&spec.image) {
        "    pull_policy: never\n"
    } else {
        ""
    };
    // The token sits at the start of its line and carries its own newline, so
    // an image that needs no platform pin leaves neither a comment nor a blank
    // line behind.
    // An amd64-only image is pinned to that platform so an arm64 host pulls
    // the amd64 variant and runs it under emulation.
    let platform_line = match spec.platform.as_deref() {
        Some(p) => format!("    platform: {p}\n"),
        None => String::new(),
    };
    // chap-core only checks the key when it is configured, so the default is
    // the commented pair from chap-core's own compose.ewars.yml. With
    // authentication on, the value is left for compose to substitute: it reads
    // the `.env` next to these files, so the key never has to be copied into
    // `.varde/` or into an overlay.
    let registration_key_lines = if spec.registration_key {
        "      SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}\n"
    } else {
        concat!(
            "      # Uncomment if chap has SERVICEKIT_REGISTRATION_KEY set:\n",
            "      # SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}\n"
        )
    };
    // `$$register` in the URLs below is compose's escape for a literal `$`.
    //
    // Without chap-core there is nothing to register with and nothing to wait
    // for. servicekit skips registration when SERVICEKIT_ORCHESTRATOR_URL is
    // unset, so the whole block goes rather than pointing at a missing host.
    //
    // A chap-core elsewhere is registered with over the host gateway, and it
    // calls the model back at the host and published port the overlay names,
    // since `http://<service_id>:8000` means nothing outside this network.
    let (environment_lines, chap_depends) = if let Some(external) = &spec.external_chap_core {
        // `SERVICEKIT_PORT` is the port the model registers, which chap-core
        // calls back on the host. `PORT` moves the app itself to that port,
        // and only an image that reads it gets it: one that fixes `--port
        // 8000` listens there, and its host port maps to 8000.
        let port_line = spec
            .host_port
            .map(|port| match spec.reads_port {
                true => format!(
                    "      PORT: \"{port}\"\n\
                     \x20     SERVICEKIT_PORT: \"{port}\"\n"
                ),
                false => format!("      SERVICEKIT_PORT: \"{port}\"\n"),
            })
            .unwrap_or_default();
        (
            format!(
                "    environment:\n\
                 \x20     SERVICEKIT_ORCHESTRATOR_URL: {}/v2/services/$$register\n\
                 \x20     SERVICEKIT_HOST: {}\n\
                 {port_line}{registration_key_lines}\
                 \x20   extra_hosts:\n\
                 \x20     - \"{}:host-gateway\"\n",
                external.register_url,
                external.models_host,
                crate::components::HOST_GATEWAY
            ),
            String::new(),
        )
    } else if spec.standalone {
        (String::new(), String::new())
    } else {
        (
            format!(
                "    environment:\n\
                 \x20     SERVICEKIT_ORCHESTRATOR_URL: http://chap:8000/v2/services/$$register\n\
                 {registration_key_lines}"
            ),
            "      chap:\n        condition: service_healthy\n".to_string(),
        )
    };
    let port_lines = port_lines(spec);
    // The init container chowns the data volume from busybox, which knows none
    // of the images' account names; an unresolvable one falls back to the
    // chapkit ids (`varde sync` warns about it).
    let uid_gid = overrides::chown_pair(&spec.user);
    // A `user:` line only exists to override what the image declares, so an
    // image that already runs as root gets none: overriding root with root
    // says nothing, and a `user:` that later moves off root by hand is able to
    // take away a permission the image needs.
    let user_line = match overrides::is_root(&spec.user) {
        true => String::new(),
        // The numeric form, so this line and the chown below are the same two
        // numbers; a name nothing could resolve is kept as it is, which is
        // what `varde sync` warns about.
        false => format!(
            "    user: {}\n",
            overrides::numeric_pair(&spec.user).unwrap_or_else(|| spec.user.clone())
        ),
    };
    // The init container is rendered for every model, root included, and that
    // is deliberate rather than tidy. A volume docker creates is root-owned,
    // so a root image needs no chown on a fresh one - but a volume that
    // already exists carries whoever owned it last. A deployment rendered by
    // a `varde` whose table said `1000:1000` for this model has a volume
    // chowned to 1000, and the same model re-rendered as root cannot write to
    // it: the overlay drops every capability, and `CAP_DAC_OVERRIDE` is the
    // one that lets root ignore the permission bits. One busybox one-shot
    // costs a second on `varde up` and makes that upgrade heal itself.
    //
    // The chown is recursive for the same reason. Handing over the directory
    // alone is enough for a volume docker has just created, because there is
    // nothing in it; on one that already holds a `chapkit.db` owned by
    // somebody else, the model can create files beside it and still not open
    // it, which is a service that starts, registers and fails its first job.
    // The model and its init container carry the same labels, so a tool that
    // lists a model's containers by its id finds the one-shot as well.
    let labels = labels_block(ROLE_MODEL, Some(&spec.id), spec.group.as_deref());
    let init_depends = format!(
        "      {}-init:\n        condition: service_completed_successfully\n",
        spec.service_id
    );
    let init_service = format!(
        "\n{}\n",
        fill(
            &OVERLAY_INIT_TEMPLATE,
            &[
                ("SERVICE_ID", &spec.service_id),
                ("LABELS", &labels),
                ("UID_GID", &uid_gid),
                ("DATA_DIR", &spec.data_dir),
                ("VOLUME", &spec.volume_name),
            ],
        )
    );
    fill(
        &OVERLAY_TEMPLATE,
        &[
            ("SERVICE_ID", &spec.service_id),
            ("LABELS", &labels),
            ("ID", &spec.id),
            ("VERSION", &spec.version),
            ("REPOSITORY", &spec.repository),
            ("TAG_VAR", &spec.tag_env_var),
            ("IMAGE_TAG", &spec.image_tag),
            ("IMAGE", &spec.image),
            // A digest carries its own `@`, so the reference is joined with
            // nothing rather than with a colon.
            ("TAG_SEP", crate::compose::tag_separator(&spec.image_tag)),
            ("PLATFORM_LINE", &format!("{pull_line}{platform_line}")),
            ("PORT_LINES", &port_lines),
            ("ENVIRONMENT_LINES", &environment_lines),
            ("CHAP_DEPENDS", &chap_depends),
            ("DATA_DIR", &spec.data_dir),
            ("USER_LINE", &user_line),
            ("INIT_DEPENDS", &init_depends),
            ("INIT_SERVICE", &init_service),
            ("VOLUME", &spec.volume_name),
        ],
    )
}

/// The `expose:` (and, when one was asked for, `ports:`) block of one model
/// overlay, indented for the service and ending in its own newline.
///
/// A model needs no host port to work: chap-core reaches it at
/// `http://<service_id>:8000` on the compose default network, which is the URL
/// the service registers, and a human reaches it through chap-core's read-only
/// proxy. `expose` documents the container port without asking the host for
/// anything. A published port is for people - `curl`, the model's own `/docs` -
/// and is opt-in per model. See `docs/ports.md`.
///
/// A model registered with a chap-core elsewhere whose image reads `PORT`
/// listens on its host port inside the container as well (see
/// [`container_port`]), so the mapping is that port on both sides.
fn port_lines(spec: &OverlaySpec) -> String {
    let inside = container_port(spec);
    let mut out = format!("    expose:\n      - \"{inside}\"\n");
    if let Some(port) = spec.host_port {
        let host = match spec.bind {
            Some(std::net::IpAddr::V4(addr)) => format!("{addr}:"),
            Some(std::net::IpAddr::V6(addr)) => format!("[{addr}]:"),
            None => String::new(),
        };
        out.push_str(&format!("    ports:\n      - \"{host}{port}:{inside}\"\n"));
    }
    out
}

/// The port the model listens on inside its container.
///
/// 8000, except for a model registered with a chap-core elsewhere whose image
/// reads `PORT`: that model listens on its host port.
///
/// Before servicekit registers, it checks that the app answers inside the
/// container. servicekit 2 checks only `SERVICEKIT_PORT`, the host port, so
/// its app has to listen there, and it is told through `PORT`. servicekit 3
/// also checks 8000, so an image that fixes `--port 8000` (every marketplace
/// model but one) stays on 8000, with its host port mapped to it: `PORT`
/// would not move it, and a mapping to the host port would reach nothing.
fn container_port(spec: &OverlaySpec) -> u16 {
    match (&spec.external_chap_core, spec.host_port) {
        (Some(_), Some(port)) if spec.reads_port => port,
        _ => 8000,
    }
}
