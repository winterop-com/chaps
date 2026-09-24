//! What `chaps models add` accepts, and what it derives from it.
//!
//! Two forms, and only two: a GitHub repository URL, which follows the
//! default branch, and an image reference on ghcr, which pins exactly. A bare
//! image name (`chapkit_ghr_model`) is refused rather than guessed at, and so
//! is a registry this CLI cannot read anonymously.

use crate::error::Result;

/// The container registry the CHAP models are published to, and the only one
/// `models add` reads.
pub const GHCR_HOST: &str = "ghcr.io";
/// Host of the repository URLs `models add` accepts.
pub const GITHUB_HOST: &str = "github.com";

/// A GitHub repository: the owner and the repository name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub owner: String,
    pub repo: String,
}

impl Repo {
    /// `<owner>/<repo>`, the path both the API and ghcr use.
    pub fn path(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }

    /// The repository's web URL, as recorded in `models-manual.yaml`.
    pub fn url(&self) -> String {
        format!("https://{GITHUB_HOST}/{}", self.path())
    }

    /// Where the repository's publish workflow pushes its images.
    ///
    /// Every model repository under `chap-models` publishes to
    /// `ghcr.io/<owner>/<repo>`, which is what the marketplace entries point
    /// at too. Image names are lowercase, whatever the repository is called.
    pub fn image(&self) -> String {
        format!("{GHCR_HOST}/{}", self.path().to_lowercase())
    }
}

/// An image reference split into the part that never moves and the pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Tagless reference, lowercase: `ghcr.io/chap-models/chapkit_ghr_model`.
    pub image: String,
    /// `sha-b1d6c31`, or `@sha256:<64 hex>` for a digest.
    pub tag: String,
}

impl Image {
    /// The reference as docker would be handed it.
    pub fn reference(&self) -> String {
        crate::compose::image_ref(&self.image, &self.tag)
    }

    /// The `<owner>/<repo>` part, which is what a ghcr token is scoped to.
    pub fn repository(&self) -> &str {
        self.image
            .strip_prefix(&format!("{GHCR_HOST}/"))
            .unwrap_or(&self.image)
    }
}

/// What the caller named on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A repository: the newest commit of its default branch that has a
    /// published `sha-` build.
    Repo(Repo),
    /// An image reference, pinned exactly as given.
    Image(Image),
}

impl Source {
    /// Parse a repository URL or an image reference.
    pub fn parse(text: &str) -> Result<Source> {
        let text = text.trim();
        if text.is_empty() {
            return Err(anyhow::anyhow!(
                "name a GitHub repository URL or a ghcr image reference"
            ));
        }
        if is_github(text) {
            return parse_repo(text).map(Source::Repo);
        }
        parse_image(text).map(Source::Image)
    }

    /// The tagless image this source publishes to.
    pub fn image(&self) -> String {
        match self {
            Source::Repo(repo) => repo.image(),
            Source::Image(image) => image.image.clone(),
        }
    }

    /// The repository URL, for a source that names one.
    pub fn repository(&self) -> Option<String> {
        match self {
            Source::Repo(repo) => Some(repo.url()),
            Source::Image(_) => None,
        }
    }

    /// The id this source is recorded under unless `--id` says otherwise.
    ///
    /// The repository name (or the last path segment of the image), folded to
    /// lowercase with everything outside `[a-z0-9]` written as `_` - the shape
    /// every marketplace id already has.
    pub fn default_id(&self) -> String {
        let last = match self {
            Source::Repo(repo) => repo.repo.clone(),
            Source::Image(image) => image
                .image
                .rsplit('/')
                .next()
                .unwrap_or(&image.image)
                .to_string(),
        };
        snake(&last)
    }

    /// How the entry's summary names where it came from.
    pub fn describe(&self) -> String {
        match self {
            Source::Repo(repo) => repo.url(),
            Source::Image(image) => image.reference(),
        }
    }
}

/// Whether this looks like a GitHub repository rather than an image.
fn is_github(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.starts_with("git@github.com:")
        || lower.starts_with(GITHUB_HOST)
        || lower.starts_with("www.github.com")
        || lower.contains("://github.com/")
        || lower.contains("://www.github.com/")
}

/// `https://github.com/<owner>/<repo>` and the spellings around it.
///
/// A URL copied out of a browser carries more than the two segments
/// (`/tree/main`, `/blob/...`); everything after the repository is dropped,
/// because the repository is the whole of what is being named.
fn parse_repo(text: &str) -> Result<Repo> {
    let rest = text
        .strip_prefix("git@github.com:")
        .map(str::to_string)
        .unwrap_or_else(|| {
            let without_scheme = text
                .split_once("://")
                .map(|(_, rest)| rest)
                .unwrap_or(text)
                .trim_start_matches("www.");
            without_scheme
                .strip_prefix(GITHUB_HOST)
                .unwrap_or(without_scheme)
                .trim_start_matches('/')
                .to_string()
        });
    let rest = rest.trim_end_matches('/');
    let mut parts = rest.split('/').filter(|p| !p.is_empty());
    let owner = parts.next().unwrap_or_default().to_string();
    let repo = parts
        .next()
        .unwrap_or_default()
        .trim_end_matches(".git")
        .to_string();
    if owner.is_empty() || repo.is_empty() {
        return Err(anyhow::anyhow!(
            "`{text}` names no repository; the form is https://github.com/<owner>/<repo>"
        ));
    }
    Ok(Repo { owner, repo })
}

/// `ghcr.io/<owner>/<repo>:<tag>` or the same with `@sha256:<digest>`.
fn parse_image(text: &str) -> Result<Image> {
    let host = text.split('/').next().unwrap_or_default();
    if !text.contains('/') || !host.contains('.') {
        return Err(anyhow::anyhow!(
            "`{text}` is neither a repository URL nor a registry image reference; \
             pass https://github.com/<owner>/<repo> or {GHCR_HOST}/<owner>/<repo>:<tag>"
        ));
    }
    if !host.eq_ignore_ascii_case(GHCR_HOST) {
        return Err(anyhow::anyhow!(
            "`{text}` is on {host}; `chaps models add` reads {GHCR_HOST} images, \
             so enable it from there or add the model to the marketplace"
        ));
    }

    if let Some((image, digest)) = text.split_once('@') {
        let digest = digest.trim();
        if !is_digest(digest) {
            return Err(anyhow::anyhow!(
                "`{digest}` is not a digest; the form is @sha256:<64 hex characters>"
            ));
        }
        return Ok(Image {
            image: image.trim_end_matches(':').to_lowercase(),
            tag: format!("@{digest}"),
        });
    }

    // A colon inside the host's port (`host:5000/x`) is not a tag, so only a
    // colon after the last `/` counts.
    let last_slash = text.rfind('/').map(|i| i + 1).unwrap_or(0);
    match text[last_slash..].find(':') {
        Some(i) => {
            let (image, tag) = text.split_at(last_slash + i);
            let tag = &tag[1..];
            if tag.is_empty() {
                return Err(anyhow::anyhow!("`{text}` ends in an empty tag"));
            }
            Ok(Image {
                image: image.to_lowercase(),
                tag: tag.to_string(),
            })
        }
        None => Err(anyhow::anyhow!(
            "`{text}` names no tag, so there is nothing to pin; \
             add `:<tag>` or `@sha256:<digest>`"
        )),
    }
}

/// Whether `text` is a `sha256:<64 hex>` digest.
fn is_digest(text: &str) -> bool {
    match text.split_once(':') {
        Some(("sha256", hex)) => hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()),
        _ => false,
    }
}

/// `GHRmodel-chapkit` -> `ghrmodel_chapkit`: the shape of a marketplace id.
pub fn snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

/// The compose service name an id gets unless `--service-id` says otherwise:
/// the id with its underscores written as hyphens, as the marketplace does.
pub fn default_service_id(id: &str) -> String {
    id.replace('_', "-")
}

/// Whether `id` is usable as a key in `.chaps/models-manual.yaml`.
///
/// The same alphabet the marketplace uses, because the id names a volume
/// (`ck_<id>_data`) and an environment variable (`<ID>_IMAGE_TAG`).
pub fn check_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(anyhow::anyhow!("the id is empty"));
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(anyhow::anyhow!(
            "`{id}` is not a usable id; use lowercase letters, digits and underscores"
        ));
    }
    Ok(())
}

/// Whether `service_id` is usable as a compose service and DNS name.
pub fn check_service_id(service_id: &str) -> Result<()> {
    if service_id.is_empty() {
        return Err(anyhow::anyhow!("the service id is empty"));
    }
    let first = service_id.chars().next().unwrap_or('-');
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(anyhow::anyhow!(
            "`{service_id}` is not a usable service id; it must start with a letter or a digit"
        ));
    }
    if !service_id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(anyhow::anyhow!(
            "`{service_id}` is not a usable service id; use lowercase letters, digits and hyphens"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_of(text: &str) -> Repo {
        match Source::parse(text).unwrap_or_else(|e| panic!("{text}: {e}")) {
            Source::Repo(repo) => repo,
            other => panic!("{text} parsed as {other:?}"),
        }
    }

    fn image_of(text: &str) -> Image {
        match Source::parse(text).unwrap_or_else(|e| panic!("{text}: {e}")) {
            Source::Image(image) => image,
            other => panic!("{text} parsed as {other:?}"),
        }
    }

    #[test]
    fn every_spelling_of_a_repository_url_is_the_same_repository() {
        for text in [
            "https://github.com/chap-models/chapkit_ghr_model",
            "https://github.com/chap-models/chapkit_ghr_model/",
            "https://github.com/chap-models/chapkit_ghr_model.git",
            "http://github.com/chap-models/chapkit_ghr_model",
            "https://www.github.com/chap-models/chapkit_ghr_model",
            "github.com/chap-models/chapkit_ghr_model",
            "git@github.com:chap-models/chapkit_ghr_model.git",
            // A URL copied out of a browser, tree view and all.
            "https://github.com/chap-models/chapkit_ghr_model/tree/main",
        ] {
            let repo = repo_of(text);
            assert_eq!(repo.owner, "chap-models", "{text}");
            assert_eq!(repo.repo, "chapkit_ghr_model", "{text}");
            assert_eq!(
                repo.url(),
                "https://github.com/chap-models/chapkit_ghr_model"
            );
            assert_eq!(
                repo.image(),
                "ghcr.io/chap-models/chapkit_ghr_model",
                "{text}"
            );
        }
    }

    #[test]
    fn a_repository_derives_the_id_and_the_service_id() {
        let source = Source::parse("https://github.com/chap-models/chapkit_ghr_model").unwrap();
        assert_eq!(source.default_id(), "chapkit_ghr_model");
        assert_eq!(
            default_service_id(&source.default_id()),
            "chapkit-ghr-model"
        );
        assert_eq!(
            source.repository().as_deref(),
            Some("https://github.com/chap-models/chapkit_ghr_model")
        );

        // A repository nobody named in snake_case still yields a usable id.
        let source = Source::parse("https://github.com/someone/GHR-Model.v2").unwrap();
        assert_eq!(source.default_id(), "ghr_model_v2");
        check_id(&source.default_id()).unwrap();
        check_service_id(&default_service_id(&source.default_id())).unwrap();
    }

    #[test]
    fn an_image_reference_splits_into_the_image_and_the_pin() {
        let image = image_of("ghcr.io/chap-models/chapkit_ghr_model:sha-1eb8cf1");
        assert_eq!(image.image, "ghcr.io/chap-models/chapkit_ghr_model");
        assert_eq!(image.tag, "sha-1eb8cf1");
        assert_eq!(image.repository(), "chap-models/chapkit_ghr_model");
        assert_eq!(
            image.reference(),
            "ghcr.io/chap-models/chapkit_ghr_model:sha-1eb8cf1"
        );

        // The image name is lowercased; the tag is left as it was typed.
        let image = image_of("ghcr.io/Chap-Models/Chapkit_GHR_Model:Sha-1eb8cf1");
        assert_eq!(image.image, "ghcr.io/chap-models/chapkit_ghr_model");
        assert_eq!(image.tag, "Sha-1eb8cf1");
    }

    #[test]
    fn a_digest_keeps_its_own_separator() {
        let digest = "a".repeat(64);
        let image = image_of(&format!(
            "ghcr.io/chap-models/chapkit_ghr_model@sha256:{digest}"
        ));
        assert_eq!(image.image, "ghcr.io/chap-models/chapkit_ghr_model");
        assert_eq!(image.tag, format!("@sha256:{digest}"));
        assert_eq!(
            image.reference(),
            format!("ghcr.io/chap-models/chapkit_ghr_model@sha256:{digest}")
        );
        assert_eq!(
            Source::Image(image).default_id(),
            "chapkit_ghr_model",
            "the id comes from the image path, not the pin"
        );
    }

    #[test]
    fn what_cannot_be_added_says_why() {
        for (text, needle) in [
            ("", "GitHub repository URL"),
            ("chapkit_ghr_model", "neither a repository URL"),
            ("docker.io/library/nginx:1", "chaps models add` reads"),
            ("ghcr.io/chap-models/chapkit_ghr_model", "names no tag"),
            ("ghcr.io/chap-models/chapkit_ghr_model:", "empty tag"),
            ("ghcr.io/x/y@sha256:nope", "not a digest"),
            ("https://github.com/chap-models", "names no repository"),
        ] {
            let err = Source::parse(text).expect_err(text);
            assert!(err.to_string().contains(needle), "{text}: {err}");
        }
    }

    #[test]
    fn an_id_and_a_service_id_are_checked_against_what_names_a_volume() {
        check_id("chapkit_ghr_model").unwrap();
        check_id("m2").unwrap();
        for bad in ["", "Chapkit", "chapkit-ghr", "chap kit", "chap.kit"] {
            assert!(check_id(bad).is_err(), "{bad}");
        }

        check_service_id("chapkit-ghr-model").unwrap();
        for bad in ["", "-model", "Chapkit", "chapkit_ghr", "chap model"] {
            assert!(check_service_id(bad).is_err(), "{bad}");
        }
    }
}
