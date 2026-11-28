//! Public installation probes using a synthetic Node process.
#![cfg(unix)]

use velune_agent_runtime::version::probe_installation;
use velune_test_support::TempRoot;

#[test]
fn pi_installation_probes_once_and_keeps_resolved_path_on_version_failure() {
    let root = TempRoot::new("pi-installation-probe");
    let launcher = root.join("pi-launcher");
    let cli = root.join("pi.js");
    let node = root.executable("node", include_str!("fixtures/pi_version_peer.py"));

    let installed = probe_installation("pi", &launcher, Some(&node));
    assert_eq!(installed.binary, cli);
    assert_eq!(installed.version.unwrap(), "1.4.0");

    root.touch("reject");
    let unavailable = probe_installation("pi", &launcher, Some(&node));
    assert_eq!(unavailable.binary, cli);
    assert_eq!(
        unavailable.version.unwrap_err().code(),
        "version_probe_failed"
    );
    assert_eq!(root.read("calls"), "resolve\nversion\nresolve\nversion\n");
}
