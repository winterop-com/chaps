//! Helpers shared by the unit tests of several modules.

/// A port on 127.0.0.1 that nothing listens on: one the kernel handed out and
/// that was released at once. Not port 9, which a machine with the discard
/// service enabled answers on.
pub fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
    listener.local_addr().expect("a local address").port()
}

/// `http://127.0.0.1:<closed port>`, for a request that must fail to connect.
pub fn unreachable_base() -> String {
    format!("http://127.0.0.1:{}", closed_port())
}

/// [`unreachable_base`], the same for the whole test run: for the tests that
/// key a cache by the URL and build their options more than once.
pub fn shared_unreachable_base() -> &'static str {
    static BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BASE.get_or_init(unreachable_base)
}
