use super::{connect_start_command, inspect_explicit_provider_grant, less_ready_status};

#[test]
fn explicit_provider_grant_readiness_is_scope_bound() {
    let ready = inspect_explicit_provider_grant(
        "grant_google",
        &["properties.read".to_owned(), "reports.read".to_owned()],
        &["properties.read".to_owned()],
    );
    assert_eq!(ready.status, "ready");

    let missing = inspect_explicit_provider_grant(
        "grant_google",
        &["properties.read".to_owned()],
        &["reports.read".to_owned()],
    );
    assert_eq!(missing.status, "needs_provider_grant");
}

#[test]
fn readiness_keeps_the_most_actionable_blocker() {
    assert_eq!(
        less_ready_status("provider_readiness_unknown", "needs_provider_grant"),
        "needs_provider_grant"
    );
    assert_eq!(
        less_ready_status("needs_provider_grant", "ready"),
        "needs_provider_grant"
    );
}

#[test]
fn connect_setup_command_preserves_exact_provider_scopes() {
    assert_eq!(
        connect_start_command(
            "google-search-console",
            &["sites.read".to_owned(), "url.inspect".to_owned()]
        ),
        "runx connect start google-search-console --scope sites.read --scope url.inspect"
    );
}
