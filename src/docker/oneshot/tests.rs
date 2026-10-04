use super::*;

#[test]
fn the_untagged_images_are_left_out_of_the_tags() {
    let listed = "v2.3.1\nmaster\n<none>\n\nlatest\n";
    assert_eq!(parse_tags(listed), ["v2.3.1", "master", "latest"]);
    assert!(parse_tags("").is_empty());
}
