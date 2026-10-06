//! The seed a `dhis2` component's database is restored from, and the dumps varde knows of.

use super::*;

/// The minor line of a DHIS2 tag: everything up to the second dot.
///
/// `2.42`, `2.42.1` and `2.42.1.1` are all the `2.42` line, which is what
/// [`DHIS2_SEED_DUMPS`] is keyed on. A tag with fewer than two dots is its own
/// key, so a `dev` or a `latest` someone pinned answers rather than panicking.
pub fn dhis2_minor(tag: &str) -> &str {
    let tag = tag.trim();
    let mut dots = tag.match_indices('.');
    match (dots.next(), dots.next()) {
        (Some(_), Some((at, _))) => &tag[..at],
        _ => tag,
    }
}

/// The dump a pinned tag is seeded from, or `None` when varde knows of none for
/// that minor line.
pub fn dhis2_seed_dump(tag: &str) -> Option<&'static str> {
    let minor = dhis2_minor(tag);
    DHIS2_SEED_DUMPS
        .iter()
        .find(|(line, _)| *line == minor)
        .map(|(_, url)| *url)
}

/// What a `dhis2` component's database is restored from the first time it is
/// created.
///
/// Three answers, written in `components.yaml` with the same words
/// `--dhis2-seed` takes: `default`, `none`, or the dump itself. A dump is a URL
/// the one-shot downloads or a path in the project directory it reads, and the
/// value decides which rather than a second setting: an `http://` or `https://`
/// prefix is the whole of the rule.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Dhis2Seed {
    /// The dump the pinned minor line publishes, from [`DHIS2_SEED_DUMPS`].
    #[default]
    Default,
    /// An empty database, which DHIS2 migrates itself on the first start.
    None,
    /// This dump, whatever the tag says.
    From(String),
}

impl Dhis2Seed {
    /// The value as `components.yaml` writes it and `--dhis2-seed` takes it.
    pub fn as_str(&self) -> &str {
        match self {
            Dhis2Seed::Default => "default",
            Dhis2Seed::None => "none",
            Dhis2Seed::From(source) => source,
        }
    }

    /// The seed one written value names.
    ///
    /// The two words are matched trimmed and without case, because they arrive
    /// from a command line as well as from YAML, and an empty value is the
    /// default rather than a dump whose name is nothing.
    pub fn parse(value: &str) -> Dhis2Seed {
        let value = value.trim();
        match value.to_ascii_lowercase().as_str() {
            "" | "default" => Dhis2Seed::Default,
            "none" => Dhis2Seed::None,
            _ => Dhis2Seed::From(value.to_string()),
        }
    }

    /// Whether this seed is a URL something has to download, which is the one
    /// shape `--offline` refuses.
    pub fn is_url(&self) -> bool {
        matches!(self, Dhis2Seed::From(source)
            if source.starts_with("http://") || source.starts_with("https://"))
    }
}

impl Serialize for Dhis2Seed {
    /// One string, so the file reads as `seed: default` rather than as a tagged
    /// enum nobody would type by hand.
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Dhis2Seed {
    /// An `Option<String>` rather than a `String`, so a hand-edited `seed:` with
    /// nothing after it is the default rather than a type error: YAML reads that
    /// as null, and "no value" is exactly what the default means.
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Dhis2Seed, D::Error> {
        Ok(match Option::<String>::deserialize(deserializer)? {
            Some(value) => Dhis2Seed::parse(&value),
            None => Dhis2Seed::Default,
        })
    }
}
