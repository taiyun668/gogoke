#[path = "../src/release_policy.rs"]
mod release_policy;

#[test]
fn authenticated_loopback_is_the_only_legacy_tcp_exception() {
    for endpoint in ["127.0.0.1:4732", "[::1]:4732"] {
        release_policy::validate_legacy_loopback(endpoint, Some("token"), false).unwrap();
    }
    for endpoint in [
        "0.0.0.0:4732",
        "[::]:4732",
        "host.example:4732",
        "localhost:4732",
        "127.0.0.1:0",
        "[::1]:0",
    ] {
        assert!(release_policy::validate_legacy_loopback(endpoint, Some("token"), false).is_err());
    }
    assert!(release_policy::validate_legacy_loopback("127.0.0.1:4732", None, false).is_err());
    assert!(
        release_policy::validate_legacy_loopback("127.0.0.1:4732", Some("token"), true).is_err()
    );
}

#[test]
fn sealed_features_reject_before_side_effect_calls() {
    assert!(release_policy::require_voice("microphone").is_err());
    assert!(release_policy::require_remote_external("remote daemon").is_err());

    let dictation = include_str!("../src/dictation/real.rs");
    assert_before(
        dictation,
        "require_voice(\"dictation model download\")",
        "reqwest::Client::builder",
    );
    assert_before(
        dictation,
        "require_voice(\"microphone capture\")",
        "default_input_device",
    );
    assert_before(
        dictation,
        "require_voice(\"microphone permission request\")",
        "request_microphone_permission",
    );

    let tailscale = include_str!("../src/tailscale/daemon_commands.rs");
    assert_before(
        tailscale,
        "require_remote_external(\"mobile access daemon start\")",
        "resolve_daemon_binary_path",
    );
}

#[test]
fn direct_daemon_and_daemonctl_are_loopback_gated() {
    let daemon = include_str!("../src/bin/gogoke_daemon.rs");
    assert!(daemon.contains("127.0.0.1:4732"));
    assert_before(daemon, "validate_legacy_loopback", "Ok(DaemonConfig");

    let daemonctl = include_str!("../src/bin/gogoke_daemonctl.rs");
    assert!(daemonctl.contains("127.0.0.1:4732"));
    assert_before(daemonctl, "validate_legacy_loopback", "match args.command");
    assert!(daemonctl.contains(
        "Refusing shutdown because authenticated managed daemon ownership could not be verified"
    ));
}

fn assert_before(text: &str, guard: &str, side_effect: &str) {
    let guard_index = text
        .find(guard)
        .unwrap_or_else(|| panic!("missing guard: {guard}"));
    let effect_index = text[guard_index + guard.len()..]
        .find(side_effect)
        .map(|relative| guard_index + guard.len() + relative)
        .unwrap_or_else(|| panic!("missing side effect after guard: {side_effect}"));
    assert!(
        guard_index < effect_index,
        "guard {guard} must precede {side_effect}"
    );
}
