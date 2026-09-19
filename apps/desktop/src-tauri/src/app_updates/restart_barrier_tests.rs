use super::*;
use std::future::{Future, poll_fn};
use std::task::Poll;
use tokio::sync::oneshot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExitCall {
    Ordinary,
    Restart,
}

#[derive(Default)]
struct ExitRecorder(Mutex<Vec<ExitCall>>);
impl ApplicationExit for ExitRecorder {
    fn ordinary_exit(&self) {
        self.0.lock().unwrap().push(ExitCall::Ordinary);
    }
    fn final_restart(&self) {
        self.0.lock().unwrap().push(ExitCall::Restart);
    }
}

#[tokio::test]
async fn restart_is_not_requested_until_updater_and_core_have_settled() {
    let quit_stage = AtomicU8::new(QuitStage::Idle.repr());
    let restart = AtomicBool::new(false);
    let exit = ExitRecorder::default();
    request_application_restart_with(&quit_stage, &restart, &exit);
    assert_eq!(*exit.0.lock().unwrap(), vec![ExitCall::Ordinary]);
    assert!(claim_core_shutdown(&quit_stage));
    let (updater_tx, updater_rx) = oneshot::channel();
    let (core_tx, core_rx) = oneshot::channel();
    let core_started = std::sync::atomic::AtomicBool::new(false);
    let mut finish = Box::pin(finish_application_shutdown(
        &quit_stage,
        &restart,
        async {
            updater_rx.await.unwrap();
        },
        async {
            core_started.store(true, Ordering::Release);
            core_rx.await.unwrap();
            CoreExitOutcome::Completed
        },
        &exit,
    ));
    assert!(poll_fn(|cx| Poll::Ready(finish.as_mut().poll(cx).is_pending())).await);
    assert!(!core_started.load(Ordering::Acquire));
    assert_eq!(*exit.0.lock().unwrap(), vec![ExitCall::Ordinary]);
    updater_tx.send(()).unwrap();
    assert!(poll_fn(|cx| Poll::Ready(finish.as_mut().poll(cx).is_pending())).await);
    assert!(core_started.load(Ordering::Acquire));
    assert_eq!(*exit.0.lock().unwrap(), vec![ExitCall::Ordinary]);
    core_tx.send(()).unwrap();
    finish.await;
    assert_eq!(
        *exit.0.lock().unwrap(),
        vec![ExitCall::Ordinary, ExitCall::Restart]
    );
}

#[tokio::test]
async fn core_exit_waits_for_actor_completion_not_just_command_submission() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = CoreRuntime::start_with_data_dir(directory.path().to_owned());
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        stop_core_for_exit(&runtime),
    )
    .await
    .unwrap();
    assert_eq!(outcome, CoreExitOutcome::Completed);
    // The acknowledged cleanup barrier is the contract, not the scheduler's
    // moment of marking the enclosing JoinHandle finished.
    assert!(runtime.wait_for_shutdown().await.is_ok());
    runtime.shutdown().await;
}

#[tokio::test]
async fn simultaneous_quit_and_restart_share_one_barrier_and_restart_once() {
    let stage = AtomicU8::new(QuitStage::Idle.repr());
    let restart = AtomicBool::new(false);
    let exit = ExitRecorder::default();
    assert!(claim_core_shutdown(&stage));
    let (updater_tx, updater_rx) = oneshot::channel();
    let (core_tx, core_rx) = oneshot::channel();
    let mut finish = Box::pin(finish_application_shutdown(
        &stage,
        &restart,
        async {
            updater_rx.await.unwrap();
        },
        async {
            core_rx.await.unwrap();
            CoreExitOutcome::Completed
        },
        &exit,
    ));
    assert!(poll_fn(|cx| Poll::Ready(finish.as_mut().poll(cx).is_pending())).await);
    assert!(exit.0.lock().unwrap().is_empty());
    // Ordinary Quit is already waiting for the updater when its installation
    // completes and asks to restart. The coordinator must retain that intent.
    request_application_restart_with(&stage, &restart, &exit);
    request_application_restart_with(&stage, &restart, &exit);
    assert!(!claim_core_shutdown(&stage));
    updater_tx.send(()).unwrap();
    assert!(poll_fn(|cx| Poll::Ready(finish.as_mut().poll(cx).is_pending())).await);
    assert_eq!(*exit.0.lock().unwrap(), vec![ExitCall::Ordinary]);
    core_tx.send(()).unwrap();
    finish.await;
    request_application_restart_with(&stage, &restart, &exit);
    assert_eq!(
        *exit.0.lock().unwrap(),
        vec![ExitCall::Ordinary, ExitCall::Restart]
    );
}

#[tokio::test]
async fn successful_ordinary_quit_does_not_request_restart() {
    let stage = AtomicU8::new(QuitStage::ShuttingDown.repr());
    let restart = AtomicBool::new(false);
    let exit = ExitRecorder::default();
    finish_application_shutdown(
        &stage,
        &restart,
        async {},
        async { CoreExitOutcome::Completed },
        &exit,
    )
    .await;
    assert_eq!(*exit.0.lock().unwrap(), vec![ExitCall::Ordinary]);
    assert_eq!(
        QuitStage::from_repr(stage.load(Ordering::Acquire)),
        QuitStage::ShutdownComplete
    );
}

#[tokio::test]
async fn failed_or_timed_out_cleanup_forces_ordinary_exit_and_cannot_late_restart() {
    for outcome in [CoreExitOutcome::Failed, CoreExitOutcome::TimedOut] {
        let stage = AtomicU8::new(QuitStage::ShuttingDown.repr());
        let restart = AtomicBool::new(true);
        let exit = ExitRecorder::default();
        finish_application_shutdown(&stage, &restart, async {}, async { outcome }, &exit).await;
        request_application_restart_with(&stage, &restart, &exit);
        assert_eq!(*exit.0.lock().unwrap(), vec![ExitCall::Ordinary]);
        assert!(!restart.load(Ordering::Acquire));
        assert_eq!(
            QuitStage::from_repr(stage.load(Ordering::Acquire)),
            QuitStage::ForcedExit
        );
        assert_eq!(
            quit_request_action(QuitStage::ForcedExit),
            QuitRequestAction::Exit
        );
        assert!(!claim_core_shutdown(&stage));
    }
}

#[tokio::test]
async fn core_deadline_bounds_pending_work_and_distinguishes_failure() {
    let deadline = std::time::Duration::from_millis(1);
    assert_eq!(
        await_core_exit(deadline, std::future::pending()).await,
        CoreExitOutcome::TimedOut
    );
    assert_eq!(
        await_core_exit(deadline, async { Err(()) }).await,
        CoreExitOutcome::Failed
    );
    assert_eq!(
        await_core_exit(deadline, async { Ok(()) }).await,
        CoreExitOutcome::Completed
    );
}
