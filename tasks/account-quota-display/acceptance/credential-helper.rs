#[path = "../../../packages/application/src/credential_helper.rs"]
mod credential_helper;
use credential_helper::{CredentialError, execute_timeout};
use std::time::Duration;
fn run(script: &str, timeout: Duration) -> Result<String, CredentialError> {
    let mut command = tokio::process::Command::new("/bin/sh");
    command.arg("-c").arg(script).env_clear();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(execute_timeout(command, timeout))
}
#[test]
fn eof_before_successful_exit() {
    assert_eq!(
        run(
            "printf quota; exec 1>&-; /bin/sleep 0.1; exit 0",
            Duration::from_secs(2)
        )
        .unwrap(),
        "quota"
    );
}
#[test]
fn eof_before_nonzero_exit() {
    assert!(matches!(
        run(
            "printf quota; exec 1>&-; /bin/sleep 0.1; exit 7",
            Duration::from_secs(2)
        ),
        Err(CredentialError::Exit { code: Some(7) })
    ));
}
#[test]
fn eof_does_not_reset_timeout() {
    assert!(matches!(
        run(
            "printf quota; exec 1>&-; /bin/sleep 10",
            Duration::from_millis(100)
        ),
        Err(CredentialError::Timeout)
    ));
}
#[test]
fn output_limit_remains_enforced() {
    assert!(matches!(
        run("/usr/bin/head -c 65537 /dev/zero", Duration::from_secs(2)),
        Err(CredentialError::InvalidOutput)
    ));
}
