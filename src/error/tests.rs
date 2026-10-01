use super::*;

#[test]
fn a_taken_port_says_who_has_it() {
    assert_eq!(
        ChapError::PortInUse {
            port: 5001,
            holder: PortHolder::ComposeFile,
        }
        .to_string(),
        "host port 5001 is already in use by another compose file"
    );
    assert_eq!(
        ChapError::PortInUse {
            port: 8000,
            holder: PortHolder::Host,
        }
        .to_string(),
        "host port 8000 is already in use by the host (something is listening)"
    );
}
