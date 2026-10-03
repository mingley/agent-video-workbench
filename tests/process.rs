use agent_video_workbench::{
    Error,
    process::{self, Control, Uncontrolled},
};
use std::{
    process::Command,
    time::{Duration, Instant},
};
struct CancelAt(Instant);
impl Control for CancelAt {
    fn check(&mut self) -> agent_video_workbench::Result<()> {
        if self.0.elapsed() > Duration::from_millis(150) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
#[test]
fn cancellation_stops_owned_process_group_and_returns_promptly() {
    let start = Instant::now();
    let result = process::run(
        Command::new("sh").args(["-c", "sleep 60 & wait"]),
        Duration::from_secs(30),
        &mut CancelAt(start),
    );
    assert!(matches!(result, Err(Error::Cancelled)));
    assert!(start.elapsed() < Duration::from_secs(3));
}
#[test]
fn timeout_and_failure_have_bounded_diagnostics() {
    assert!(matches!(
        process::run(
            Command::new("sleep").arg("5"),
            Duration::from_millis(100),
            &mut Uncontrolled
        ),
        Err(Error::Timeout)
    ));
    let result = process::run(
        Command::new("sh").args(["-c", "head -c 1048576 /dev/zero >&2; exit 1"]),
        Duration::from_secs(10),
        &mut Uncontrolled,
    );
    let Err(Error::Execution { details, .. }) = result else {
        panic!("expected failure")
    };
    assert!(details.len() <= 64 * 1024);
}
