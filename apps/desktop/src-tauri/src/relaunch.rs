//! Detached relaunch handoff for an installed update (#1864).
//!
//! macOS gives a replacement process its own Dock tile when it starts while the
//! old instance is still alive (tauri#16449), which loses the pinned tile. The
//! shutdown barrier settles the updater and Core, but that is not the same as
//! the old OS process having exited. So the replacement launch is delegated to
//! a detached helper: this binary re-executed with [`RELAUNCH_HELPER_FLAG`],
//! own process group, and null stdio. It waits for the old PID to disappear and
//! then launches the bundle through LaunchServices exactly once.
//!
//! The wait is bounded. When it expires the relaunch is abandoned instead of
//! degrading into the concurrent launch this exists to avoid, and the caller
//! exits ordinarily (`overview.md`, "Desktop Application Updates").

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

/// Bounded wait for the old process to exit before the replacement launches.
const RELAUNCH_WAIT_TIMEOUT: Duration = Duration::from_secs(15);
/// Liveness poll interval while waiting for the old process to exit.
const RELAUNCH_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// First argument that puts this same binary into detached-helper mode.
///
/// This flag and the argv layout it introduces are a cross-version protocol:
/// the *next* installed build reads them, not the one that wrote them, so a
/// rename or reshape silently breaks the relaunch for every user one version
/// back. `helper_arguments`/`from_helper_arguments` pin the shape.
const RELAUNCH_HELPER_FLAG: &str = "--koushi-relaunch-helper";

/// When to stop waiting for the old process and how often to probe it.
#[derive(Clone, Copy, Debug)]
struct RelaunchWait {
    timeout: Duration,
    poll: Duration,
}

const RELAUNCH_WAIT: RelaunchWait = RelaunchWait {
    timeout: RELAUNCH_WAIT_TIMEOUT,
    poll: RELAUNCH_POLL_INTERVAL,
};

/// The replacement launch the helper owes this process.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RelaunchPlan {
    old_pid: u32,
    bundle: PathBuf,
    args: Vec<OsString>,
}

impl RelaunchPlan {
    /// Helper argv. Paths and arguments stay separate argv entries; nothing is
    /// joined into a command line.
    fn helper_arguments(&self) -> Vec<OsString> {
        let mut arguments = Vec::with_capacity(self.args.len() + 3);
        arguments.push(OsString::from(RELAUNCH_HELPER_FLAG));
        arguments.push(OsString::from(self.old_pid.to_string()));
        arguments.push(self.bundle.clone().into_os_string());
        arguments.extend(self.args.iter().cloned());
        arguments
    }

    fn from_helper_arguments(arguments: &[OsString]) -> Option<Self> {
        let [flag, old_pid, bundle, args @ ..] = arguments else {
            return None;
        };
        if flag != RELAUNCH_HELPER_FLAG || bundle.is_empty() {
            return None;
        }
        Some(Self {
            old_pid: old_pid.to_str()?.parse().ok()?,
            bundle: PathBuf::from(bundle),
            args: args.to_vec(),
        })
    }

    /// LaunchServices argv: `open -n <bundle> --args <forwarded args>`.
    fn launch_arguments(&self) -> Vec<OsString> {
        let mut arguments = Vec::with_capacity(self.args.len() + 3);
        arguments.push(OsString::from("-n"));
        arguments.push(self.bundle.clone().into_os_string());
        arguments.push(OsString::from("--args"));
        arguments.extend(self.args.iter().cloned());
        arguments
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelaunchOutcome {
    Launched,
    TimedOut,
    LaunchFailed,
}

/// Wait for `plan.old_pid` to exit, then launch the replacement exactly once.
///
/// The probe reports whether the old process is still alive and the launcher
/// reports whether the replacement started. Both are injected so the ordering
/// and the bounded wait are testable without a real Dock.
fn relaunch_after_exit(
    plan: &RelaunchPlan,
    wait: RelaunchWait,
    probe: impl Fn(u32) -> bool,
    launch: impl FnOnce(&RelaunchPlan) -> bool,
) -> RelaunchOutcome {
    // Bounded wait: the deadline abandons the relaunch instead of launching
    // concurrently with a live old process and losing the pinned Dock tile.
    let deadline = std::time::Instant::now() + wait.timeout;
    while probe(plan.old_pid) {
        if std::time::Instant::now() >= deadline {
            return RelaunchOutcome::TimedOut;
        }
        std::thread::sleep(wait.poll);
    }
    if launch(plan) {
        RelaunchOutcome::Launched
    } else {
        RelaunchOutcome::LaunchFailed
    }
}

/// What handing the relaunch to the helper achieved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RelaunchHandoff {
    /// The detached helper owns the replacement launch.
    Detached,
    /// This process does not run from an application bundle.
    Unbundled,
    /// The helper could not be started, so nothing was launched.
    Failed,
}

fn hand_off(
    plan: Option<RelaunchPlan>,
    spawn: impl FnOnce(&RelaunchPlan) -> std::io::Result<()>,
    log_failure: impl FnOnce(&str),
) -> RelaunchHandoff {
    let Some(plan) = plan else {
        return RelaunchHandoff::Unbundled;
    };
    match spawn(&plan) {
        Ok(()) => {
            koushi_diagnostics::record(
                koushi_diagnostics::DiagnosticEvent::new(
                    koushi_diagnostics::DiagnosticLevel::Info,
                    "desktop.lifecycle",
                    "relaunch_handoff",
                )
                .field(koushi_diagnostics::DiagnosticField::token(
                    "owner",
                    "detached_helper",
                )),
            );
            RelaunchHandoff::Detached
        }
        Err(_) => {
            record_handoff_failure("helper_spawn_failed", log_failure);
            RelaunchHandoff::Failed
        }
    }
}

fn handoff_failure_event(reason: &'static str) -> koushi_diagnostics::DiagnosticEvent {
    koushi_diagnostics::DiagnosticEvent::new(
        koushi_diagnostics::DiagnosticLevel::Warn,
        "desktop.lifecycle",
        "relaunch_failed",
    )
    .field(koushi_diagnostics::DiagnosticField::token("reason", reason))
}

/// The one coarse line a handoff failure leaves behind: a reason token and
/// nothing else, so no bundle path, argv, or raw error is ever written.
fn failure_line(reason: &str) -> String {
    format!("relaunch_failed {reason}")
}

/// Durable sink for a failure raised by a process that is exiting, or by the
/// helper, which has no UI and null stdio: one token line in the macOS unified
/// log (`log show --predicate 'eventMessage CONTAINS "relaunch_failed"'`).
#[cfg(target_os = "macos")]
fn log_failure_line(line: &str) {
    let _ = std::process::Command::new("/usr/bin/logger")
        .arg("-t")
        .arg("koushi")
        .arg(line)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Records one coarse handoff failure in the diagnostic ring and on the durable
/// line. The ring dies with this process, so the line is the trace a user or
/// maintainer can actually find after "it updated and never came back".
fn record_handoff_failure(reason: &'static str, log: impl FnOnce(&str)) {
    koushi_diagnostics::record(handoff_failure_event(reason));
    log(&failure_line(reason));
}

/// `…/Koushi.app/Contents/MacOS/koushi` → `…/Koushi.app`.
fn bundle_root(executable: &Path) -> Option<&Path> {
    let macos = executable.parent()?;
    if macos.file_name()? != "MacOS" {
        return None;
    }
    let contents = macos.parent()?;
    if contents.file_name()? != "Contents" {
        return None;
    }
    contents.parent()
}

fn current_plan(executable: &Path, old_pid: u32, args: Vec<OsString>) -> Option<RelaunchPlan> {
    Some(RelaunchPlan {
        old_pid,
        bundle: bundle_root(executable)?.to_path_buf(),
        args,
    })
}

/// Whether a process exists. Signal 0 only checks existence, so `EPERM` still
/// means the old process is alive. A zombie also counts as alive: if some parent
/// does not reap the old process the helper times out and relaunches nothing,
/// which fails safe.
#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    // SAFETY: `kill` with signal 0 has no side effects and takes no pointers.
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Exit code and coarse failure reason for a helper run.
fn helper_exit(outcome: RelaunchOutcome) -> (i32, Option<&'static str>) {
    match outcome {
        RelaunchOutcome::Launched => (0, None),
        RelaunchOutcome::TimedOut => (1, Some("wait_timeout")),
        RelaunchOutcome::LaunchFailed => (1, Some("launch_failed")),
    }
}

/// Whether `final_restart` keeps Tauri's own relaunch. Only an unbundled build
/// does (and non-macOS builds, which never hand off): a failed handoff exits
/// instead of falling back to the overlapping launch this handoff exists to
/// avoid.
pub(crate) fn uses_tauri_relaunch(handoff: RelaunchHandoff) -> bool {
    handoff == RelaunchHandoff::Unbundled
}

#[cfg(target_os = "macos")]
pub(crate) fn hand_off_relaunch() -> RelaunchHandoff {
    let Ok(executable) = std::env::current_exe() else {
        record_handoff_failure("helper_spawn_failed", log_failure_line);
        return RelaunchHandoff::Failed;
    };
    let plan = current_plan(
        &executable,
        std::process::id(),
        std::env::args_os().skip(1).collect(),
    );
    hand_off(
        plan,
        |plan| spawn_helper(&executable, plan),
        log_failure_line,
    )
}

#[cfg(target_os = "macos")]
fn spawn_helper(executable: &Path, plan: &RelaunchPlan) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    // Detached: own process group and null stdio, so the helper outlives this
    // process instead of being tied to its group and open streams (#15783).
    std::process::Command::new(executable)
        .args(plan.helper_arguments())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn launch_bundle(plan: &RelaunchPlan) -> bool {
    std::process::Command::new("/usr/bin/open")
        .args(plan.launch_arguments())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Runs detached-helper mode when this process was spawned for a handoff, and
/// returns the exit code to use instead of starting the application.
#[cfg(target_os = "macos")]
pub(crate) fn run_helper_if_requested() -> Option<i32> {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    if arguments.first()? != RELAUNCH_HELPER_FLAG {
        return None;
    }
    Some(run_helper(&arguments))
}

#[cfg(target_os = "macos")]
fn run_helper(arguments: &[OsString]) -> i32 {
    let Some(plan) = RelaunchPlan::from_helper_arguments(arguments) else {
        record_handoff_failure("invalid_helper_arguments", log_failure_line);
        return 2;
    };
    let (code, reason) = helper_exit(relaunch_after_exit(
        &plan,
        RELAUNCH_WAIT,
        process_is_alive,
        launch_bundle,
    ));
    if let Some(reason) = reason {
        record_handoff_failure(reason, log_failure_line);
    }
    code
}

#[cfg(test)]
#[path = "relaunch_tests.rs"]
mod relaunch_tests;
