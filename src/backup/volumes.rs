//! Component data volumes: which ones an archive holds, and reading and
//! refilling them through a throwaway container.

use super::BUSYBOX_IMAGE;
use super::tar::read_all;
use crate::error::Result;
use std::fs::File;
use std::path::Path;
use std::process::{Command, Stdio};

/// One named volume a component keeps state in. A component with several
/// volumes has one entry here per volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentVolume {
    /// Component name, as `components.yaml` and `--with` spell it, and as
    /// [`crate::components::Component::name`] returns it. Several entries share
    /// it when one component keeps several volumes.
    pub name: &'static str,
    /// What this volume's archive member is called inside [`COMPONENTS_MEMBER`],
    /// as [`component_member`] spells it out.
    ///
    /// The component name for a component that keeps one volume, so `ocs` is
    /// `components/ocs.tar` and `s3` is `components/s3.tar`, as they have been
    /// in every archive ever written; `<name>-<what it holds>` for each volume
    /// of a component that keeps several, because two members under one name
    /// would be two entries of the same path in the tar and only the last of
    /// them would come back out. So this field is the member and `name` is the
    /// component: the manifest goes on reporting the component an operator asked
    /// for while the archive keeps one member per volume.
    ///
    /// [`COMPONENTS_MEMBER`]: super::COMPONENTS_MEMBER
    /// [`component_member`]: super::component_member
    pub member: &'static str,
    /// Compose service the volume belongs to, and the one to hold still while
    /// it is read.
    pub service: &'static str,
    /// Named volume, without the compose project prefix.
    pub volume: &'static str,
    /// Where the service mounts it. Recorded for the operator who restores by
    /// hand; the tar itself is taken relative to the volume root.
    pub data_dir: &'static str,
}

/// Every volume a component keeps state in, in [`crate::components::Component`]
/// order and, within a component, in the order
/// [`crate::components::Component::volumes`] lists them.
///
/// chap-core's state is the database and the model volumes, each captured in
/// its own right. `ocs` keeps a data directory and `s3` its object store, and
/// the `data_dir`s here are the `target:` of the volume mount in
/// `compose.ocs.yml`, `compose.s3.yml` and `compose.dhis2.yml`.
///
/// This is the list an archive is made of, which is not quite the list a
/// component *keeps*: `dhis2_dump` is declared by `compose.dhis2.yml` and named
/// by [`crate::components::Component::volumes`], and it is deliberately not here.
/// It is a download cache - the one-shot fetches the dump again when it is
/// missing - so archiving it would add the whole dump to every backup of that
/// deployment for nothing.
/// `the_table_and_the_component_agree_on_every_volume` checks the two tables
/// against each other in the direction that always holds, and pins that one
/// omission with its reason.
pub const COMPONENT_VOLUMES: &[ComponentVolume] = &[
    ComponentVolume {
        name: "ocs",
        member: "ocs",
        service: crate::compose::OCS_SERVICE,
        volume: crate::compose::render::OCS_VOLUME,
        data_dir: "/app/data",
    },
    ComponentVolume {
        name: "s3",
        member: "s3",
        service: crate::compose::S3_SERVICE,
        volume: crate::compose::render::S3_VOLUME,
        data_dir: "/data",
    },
    ComponentVolume {
        name: "dhis2",
        member: "dhis2-home",
        service: crate::compose::DHIS2_SERVICE,
        volume: crate::compose::render::DHIS2_HOME_VOLUME,
        data_dir: "/opt/dhis2",
    },
    ComponentVolume {
        name: "dhis2",
        member: "dhis2-db",
        service: "dhis2-db",
        volume: crate::compose::render::DHIS2_DB_VOLUME,
        data_dir: "/var/lib/postgresql/data",
    },
];

/// The [`COMPONENT_VOLUMES`] this deployment has enabled, one entry per volume.
pub fn component_volumes(
    components: &crate::components::Components,
) -> Vec<&'static ComponentVolume> {
    COMPONENT_VOLUMES
        .iter()
        .filter(|part| {
            crate::components::Component::from_name(part.name)
                .is_ok_and(|component| components.is_enabled(component))
        })
        .collect()
}

/// `docker run` arguments that read a named volume as a tar on stdout.
///
/// The model services carry a `<service>-init` container that mounts their
/// volume, so compose can read those without help; nothing but the component
/// itself mounts a volume in [`COMPONENT_VOLUMES`], and a container of their
/// own is the only way in. The volume is mounted at `/v` and nothing else is,
/// so an operator can run the same line by hand.
pub fn volume_read_args(volume: &str) -> Vec<String> {
    let mut args = docker_run_args(volume, false);
    args.extend(
        ["tar", "-C", "/v", "-cf", "-", "."]
            .iter()
            .map(|s| s.to_string()),
    );
    args
}

/// `docker run` arguments that empty a named volume and refill it from a tar
/// on stdin.
///
/// The three globs are what it takes to clear a directory with `sh`: `*`
/// misses dotfiles, `.[!.]*` catches them, and `..?*` catches the `..foo`
/// names neither of the others match, while none of them can ever match `.`
/// or `..` themselves. `rm` complains about the patterns that match nothing,
/// which is why only its stderr is dropped; tar's is the exit code that
/// counts.
pub fn volume_write_args(volume: &str) -> Vec<String> {
    let mut args = docker_run_args(volume, true);
    args.extend(["sh".to_string(), "-c".to_string(), refill_script("/v")]);
    args
}

/// The `sh` script that empties `dir` - dotfiles included, see
/// [`volume_write_args`] - and refills it from a tar on stdin.
///
/// One script for component volumes and model data directories alike: a
/// model restore that cleared `dir/*` alone left every hidden file written
/// since the backup in the restored volume.
pub fn refill_script(dir: &str) -> String {
    format!("rm -rf {dir}/* {dir}/.[!.]* {dir}/..?* 2>/dev/null; tar -C {dir} -xf -")
}

fn docker_run_args(volume: &str, stdin: bool) -> Vec<String> {
    let mut args = vec!["run".to_string(), "--rm".to_string()];
    if stdin {
        args.push("-i".to_string());
    }
    args.push("-v".to_string());
    args.push(format!("{volume}:/v"));
    args.push(BUSYBOX_IMAGE.to_string());
    args
}

/// Read the named volume `volume` into the file `dest`.
pub fn read_volume(volume: &str, dest: &Path) -> Result<crate::docker::Piped> {
    run_docker(&volume_read_args(volume), None, Some(dest))
}

/// Empty the named volume `volume` and unpack the tar `src` into it.
pub fn write_volume(volume: &str, src: &Path) -> Result<crate::docker::Piped> {
    run_docker(&volume_write_args(volume), Some(src), None)
}

/// Run `docker <args>` with its payload streams wired to files.
///
/// The same shape as [`crate::docker::run_compose_piped`] and for the same
/// reason: a volume tar is arbitrarily large, so it may not travel through a
/// pipe this process would have to drain. Only stderr is a pipe.
fn run_docker(
    args: &[String],
    stdin: Option<&Path>,
    stdout: Option<&Path>,
) -> Result<crate::docker::Piped> {
    crate::output::verbose(&format!("$ docker {}", args.join(" ")));
    let mut cmd = Command::new("docker");
    cmd.args(args);
    match stdin {
        Some(path) => {
            let file =
                File::open(path).map_err(|e| anyhow::anyhow!("opening {}: {e}", path.display()))?;
            cmd.stdin(Stdio::from(file));
        }
        None => {
            cmd.stdin(Stdio::null());
        }
    }
    match stdout {
        Some(path) => {
            let file = File::create(path)
                .map_err(|e| anyhow::anyhow!("creating {}: {e}", path.display()))?;
            cmd.stdout(Stdio::from(file));
        }
        None => {
            cmd.stdout(Stdio::null());
        }
    }
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("`docker` was not found on PATH")
        } else {
            anyhow::anyhow!("could not run `docker`: {e}")
        }
    })?;
    let stderr = read_all(child.stderr.take());
    let status = child
        .wait()
        .map_err(|e| anyhow::anyhow!("waiting for docker: {e}"))?;
    Ok(crate::docker::Piped {
        code: status.code().unwrap_or(1),
        stderr,
    })
}
