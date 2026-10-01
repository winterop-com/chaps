//! Reading a public image on ghcr without docker: does this tag exist, and
//! what does the image say it runs as.
//!
//! Anonymous pulls are allowed, so the whole exchange is three requests: a
//! token scoped to the repository, the manifest, and the config blob the
//! manifest points at. `chaps models add` needs this before the image is on
//! the machine, which is what makes it worth doing over HTTP rather than with
//! `docker image inspect`.

use crate::error::Result;
use serde::Deserialize;
use std::time::Duration;

/// Public ghcr, unless `CHAPS_GHCR_URL` points somewhere else.
pub const DEFAULT_URL: &str = "https://ghcr.io";

/// The manifest media types a registry may answer with. An OCI index is what
/// ghcr serves for the model images; the rest are there because a registry
/// answers with whatever the client said it understands.
const ACCEPT: &str = "application/vnd.oci.image.index.v1+json, \
     application/vnd.docker.distribution.manifest.list.v2+json, \
     application/vnd.oci.image.manifest.v1+json, \
     application/vnd.docker.distribution.manifest.v2+json";

/// The architecture a Chap model image is published for.
pub const AMD64: &str = "amd64";

/// What an image says about how it runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageConfig {
    /// `config.User`: the account the image runs as, empty for root.
    pub user: String,
    /// `config.WorkingDir`, which is where a chapkit service writes `data/`.
    pub working_dir: String,
    /// Whether the image is published for amd64 and nothing else.
    pub amd64_only: bool,
}

/// An anonymous reader of one ghcr repository.
#[derive(Debug, Clone)]
pub struct Client {
    base: String,
    repository: String,
    token: String,
    timeout: Duration,
}

impl Client {
    /// Fetch a pull token for `repository` (`<owner>/<name>`).
    ///
    /// ghcr hands anonymous pull tokens out to anyone for a public
    /// repository, and refuses for a private one - which is the same answer a
    /// missing repository gives, and both are reported as "no published
    /// build" by the caller.
    pub fn anonymous(base: &str, repository: &str, timeout: Duration) -> Result<Client> {
        let base = base.trim_end_matches('/').to_string();
        let url = format!("{base}/token?scope=repository:{repository}:pull&service=ghcr.io");
        let (status, body) = super::get(&url, timeout, &[("Accept", "application/json")])?;
        if !(200..300).contains(&status) {
            return Err(crate::error::ChapError::Http { url, status }.into());
        }
        Ok(Client {
            base,
            repository: repository.to_string(),
            token: parse_token(&body).map_err(|e| anyhow::anyhow!("reading {url}: {e}"))?,
            timeout,
        })
    }

    /// The manifest of `reference` (a tag, or `sha256:...`), or `None` when
    /// the registry has no such thing.
    pub fn manifest(&self, reference: &str) -> Result<Option<String>> {
        let reference = reference.trim_start_matches('@');
        let url = format!("{}/v2/{}/manifests/{reference}", self.base, self.repository);
        let (status, body) = super::get(
            &url,
            self.timeout,
            &[
                ("Accept", ACCEPT),
                ("Authorization", &format!("Bearer {}", self.token)),
            ],
        )?;
        match status {
            200 => Ok(Some(body)),
            404 => Ok(None),
            status => Err(crate::error::ChapError::Http { url, status }.into()),
        }
    }

    /// Whether the registry has this tag.
    pub fn tag_exists(&self, tag: &str) -> Result<bool> {
        Ok(self.manifest(tag)?.is_some())
    }

    /// One blob of this repository, as text.
    pub fn blob(&self, digest: &str) -> Result<String> {
        let url = format!("{}/v2/{}/blobs/{digest}", self.base, self.repository);
        let (status, body) = super::get(
            &url,
            self.timeout,
            &[
                ("Accept", "application/json"),
                ("Authorization", &format!("Bearer {}", self.token)),
            ],
        )?;
        if !(200..300).contains(&status) {
            return Err(crate::error::ChapError::Http { url, status }.into());
        }
        Ok(body)
    }

    /// What the image at `reference` says about its user and working
    /// directory, or `None` when there is no such image.
    ///
    /// Three hops: the index, the amd64 manifest inside it, and the config
    /// blob that manifest points at. An image published as a plain manifest
    /// rather than an index is read in two.
    pub fn image_config(&self, reference: &str) -> Result<Option<ImageConfig>> {
        let Some(top) = self.manifest(reference)? else {
            return Ok(None);
        };
        let platforms = parse_platforms(&top);
        let amd64_only = !platforms.is_empty()
            && platforms
                .iter()
                .all(|p| p.architecture == AMD64 && p.os == "linux");
        let (manifest, amd64_only) = match amd64_manifest(&top) {
            Some(digest) => match self.manifest(&digest)? {
                Some(body) => (body, amd64_only),
                None => return Ok(None),
            },
            // Not an index: the document is the manifest itself, and it says
            // nothing about which platforms exist.
            None if platforms.is_empty() => (top, false),
            None => {
                return Err(anyhow::anyhow!(
                    "{}:{reference} publishes no linux/{AMD64} image",
                    self.repository
                ));
            }
        };
        let Some(digest) = config_digest(&manifest) else {
            return Err(anyhow::anyhow!(
                "the manifest of {}:{reference} names no config blob",
                self.repository
            ));
        };
        let blob = self.blob(&digest)?;
        Ok(Some(ImageConfig {
            amd64_only,
            ..parse_config(&blob)
        }))
    }
}

/// The `token` of a ghcr token response.
pub fn parse_token(body: &str) -> Result<String> {
    #[derive(Debug, Default, Deserialize)]
    struct TokenBody {
        #[serde(default)]
        token: String,
        #[serde(default)]
        access_token: String,
    }
    let parsed: TokenBody =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("invalid token JSON: {e}"))?;
    let token = if parsed.token.trim().is_empty() {
        parsed.access_token
    } else {
        parsed.token
    };
    let token = token.trim();
    if token.is_empty() {
        return Err(anyhow::anyhow!("the registry handed out no token"));
    }
    Ok(token.to_string())
}

/// One platform an index lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub architecture: String,
    pub os: String,
    pub digest: String,
}

/// The real platforms of an index, with the attestation entries left out.
///
/// ghcr's indexes carry one `unknown/unknown` entry per real image - the
/// build provenance attestation - and taking one of those for the image is
/// how a reader ends up fetching a manifest with no config to read.
pub fn parse_platforms(body: &str) -> Vec<Platform> {
    #[derive(Debug, Default, Deserialize)]
    struct Index {
        #[serde(default)]
        manifests: Vec<Entry>,
    }
    #[derive(Debug, Default, Deserialize)]
    struct Entry {
        #[serde(default)]
        digest: String,
        #[serde(default)]
        platform: PlatformField,
    }
    #[derive(Debug, Default, Deserialize)]
    struct PlatformField {
        #[serde(default)]
        architecture: String,
        #[serde(default)]
        os: String,
    }
    let index: Index = match serde_json::from_str(body) {
        Ok(index) => index,
        Err(_) => return Vec::new(),
    };
    index
        .manifests
        .into_iter()
        .filter(|e| !e.digest.is_empty())
        .filter(|e| e.platform.architecture != "unknown" && e.platform.os != "unknown")
        .filter(|e| !e.platform.architecture.is_empty())
        .map(|e| Platform {
            architecture: e.platform.architecture,
            os: e.platform.os,
            digest: e.digest,
        })
        .collect()
}

/// The digest of the linux/amd64 manifest of an index, or `None` for a
/// document that is not an index at all.
pub fn amd64_manifest(body: &str) -> Option<String> {
    parse_platforms(body)
        .into_iter()
        .find(|p| p.architecture == AMD64 && (p.os == "linux" || p.os.is_empty()))
        .map(|p| p.digest)
}

/// The `config.digest` of one image manifest.
pub fn config_digest(body: &str) -> Option<String> {
    #[derive(Debug, Default, Deserialize)]
    struct Manifest {
        #[serde(default)]
        config: Descriptor,
    }
    #[derive(Debug, Default, Deserialize)]
    struct Descriptor {
        #[serde(default)]
        digest: String,
    }
    let manifest: Manifest = serde_json::from_str(body).ok()?;
    let digest = manifest.config.digest.trim().to_string();
    (!digest.is_empty()).then_some(digest)
}

/// `config.User` and `config.WorkingDir` of an image config blob.
pub fn parse_config(body: &str) -> ImageConfig {
    #[derive(Debug, Default, Deserialize)]
    struct Blob {
        #[serde(default)]
        config: ConfigField,
    }
    #[derive(Debug, Default, Deserialize)]
    struct ConfigField {
        #[serde(default, rename = "User")]
        user: String,
        #[serde(default, rename = "WorkingDir")]
        working_dir: String,
    }
    let blob: Blob = serde_json::from_str(body).unwrap_or_default();
    ImageConfig {
        user: blob.config.user.trim().to_string(),
        working_dir: blob.config.working_dir.trim().to_string(),
        amd64_only: false,
    }
}

#[cfg(test)]
mod tests;
