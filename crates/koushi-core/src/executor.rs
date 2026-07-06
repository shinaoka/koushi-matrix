//! Executor abstraction (Platform Portability rule 2).
//!
//! All task spawning and timing in core goes through this module. Today it
//! wraps tokio; a wasm backend swaps these implementations without touching
//! actor logic. Direct `tokio::spawn`/`tokio::time` calls elsewhere in this
//! crate are a portability violation.

use std::future::Future;
use std::time::Duration;

pub use tokio::task::JoinHandle;

pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::task::spawn(future)
}

pub fn spawn_blocking<F, R>(function: F) -> JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    tokio::task::spawn_blocking(function)
}

pub async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

pub async fn timeout<F: Future>(
    duration: Duration,
    future: F,
) -> Result<F::Output, TimeoutElapsed> {
    tokio::time::timeout(duration, future)
        .await
        .map_err(|_| TimeoutElapsed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimeoutElapsed;

#[cfg(test)]
mod tests {
    #[test]
    fn executor_exposes_blocking_task_port() {
        let source = include_str!("executor.rs");
        let public_api = source
            .split("#[cfg(test)]")
            .next()
            .expect("test module should follow public executor API");
        assert!(
            public_api.contains("pub fn spawn_blocking"),
            "blocking OS/filesystem/keyring work must go through the executor port"
        );
        assert!(
            public_api.contains("tokio::task::spawn_blocking"),
            "native executor backend must route blocking work to tokio's blocking pool"
        );
    }
}
