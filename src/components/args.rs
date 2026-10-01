//! The DHIS2 image tag and repository as given on the command line.

use super::*;

/// A DHIS2 image tag as given on the command line, or why it will not do.
/// `flag` is the flag's spelling, for the message.
pub fn dhis2_tag_arg(tag: &str, flag: &str) -> Result<String> {
    let tag = tag.trim();
    if tag.is_empty() || tag.contains(char::is_whitespace) {
        return Err(anyhow::anyhow!(
            "{flag} takes an image tag such as `2.42` or `2.43.1`"
        ));
    }
    Ok(tag.to_string())
}

/// A DHIS2 image repository as given on the command line, or why it will not
/// do. The tag has a flag of its own, so a `repo:tag` is split for the
/// operator rather than rendered as a reference with two tags.
pub fn dhis2_image_arg(image: &str, flag: &str, tag_flag: &str) -> Result<String> {
    let image = image.trim();
    let last = image.rsplit('/').next().unwrap_or(image);
    if image.is_empty() || image.contains(char::is_whitespace) || last.contains(':') {
        return Err(anyhow::anyhow!(
            "{flag} takes a repository such as `dhis2/core-dev`, and the version goes in \
             {tag_flag}: `{flag} dhis2/core-dev {tag_flag} master`"
        ));
    }
    Ok(image.to_string())
}
