use std::{future::Future, sync::Weak};

#[cfg(test)]
use super::OwnedViewScope;
use super::{Control, ScopeError};
use koushi_protocol::view::ViewRetirement;

/// Moves with the result into its completion envelope. Disarm only after the
/// actor handles that envelope, not merely after the worker sends it.
pub(crate) struct ProducerCompletion {
    control: Weak<Control>,
    completed: bool,
}

impl ProducerCompletion {
    pub(crate) fn complete(mut self) {
        self.completed = true;
        if let Some(control) = self.control.upgrade() {
            // Drop the task handle outside the lock, including for executors
            // that promptly drop an aborted future and its completion guard.
            let task = control
                .producer
                .lock()
                .expect("view producer poisoned")
                .take();
            drop(task);
        }
    }
}

impl Drop for ProducerCompletion {
    fn drop(&mut self) {
        if !self.completed
            && let Some(control) = self.control.upgrade()
        {
            control.retire(ViewRetirement::ProducerFailed);
        }
    }
}

impl Control {
    pub(super) fn spawn_producer<F, Fut>(
        self: &std::sync::Arc<Self>,
        produce: F,
    ) -> Result<(), ScopeError>
    where
        F: FnOnce(ProducerCompletion) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let retired = self.retired.lock().expect("view control poisoned");
        if retired.is_some() {
            return Err(ScopeError::Closed);
        }
        let mut producer = self.producer.lock().expect("view producer poisoned");
        if producer.is_some() {
            return Err(ScopeError::ProducerRunning);
        }
        let completion = ProducerCompletion {
            control: std::sync::Arc::downgrade(self),
            completed: false,
        };
        // Spawn does not poll inline. Keep the slot locked until installation so
        // even immediate completion cannot overtake installation of its handle.
        *producer = Some(crate::runtime::AbortOnDrop::new(crate::executor::spawn(
            async move {
                produce(completion).await;
            },
        )));
        Ok(())
    }
}

#[cfg(test)]
impl OwnedViewScope {
    pub(crate) fn spawn_producer<F, Fut>(&self, produce: F) -> Result<(), ScopeError>
    where
        F: FnOnce(ProducerCompletion) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.control.spawn_producer(produce)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view_scope_lifecycle::{ScopeDelivery, ViewScopeRegistry};
    use std::{sync::Arc, time::Duration};
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn retirement_aborts_owned_work_and_releases_task_artifacts() {
        let registry = ViewScopeRegistry::default();
        let consumer = registry
            .consumer(koushi_protocol::RuntimeConnectionId(1))
            .unwrap();
        let mut scope = consumer.open().unwrap();
        let artifact = Arc::new((
            vec![0u8; 1024],
            registry.budget.reserve_bytes(64 * 1024 * 1024).unwrap(),
        ));
        let retained = artifact.clone();
        let weak = Arc::downgrade(&artifact);
        let (started, ready) = oneshot::channel();
        let (held_reply, reply) = oneshot::channel::<()>();
        scope
            .spawn_producer(move |completion| async move {
                let _completion = completion;
                let _artifact = artifact;
                let _held_reply = held_reply;
                started.send(()).unwrap();
                std::future::pending::<()>().await;
            })
            .unwrap();
        ready.await.unwrap();
        assert_eq!(
            scope.spawn_producer(|_| async {}).err(),
            Some(ScopeError::ProducerRunning)
        );
        registry.retire(scope.id(), ViewRetirement::SourceUnavailable);
        assert!(matches!(
            scope.next_delivery().await,
            Some(ScopeDelivery::Retired(ViewRetirement::SourceUnavailable))
        ));
        assert!(
            tokio::time::timeout(Duration::from_secs(1), reply)
                .await
                .unwrap()
                .is_err()
        );
        assert!(registry.budget.reserve_bytes(200 * 1024 * 1024).is_none());
        drop(retained);
        assert!(weak.upgrade().is_none());
        assert!(registry.budget.reserve_bytes(200 * 1024 * 1024).is_some());
        assert_eq!(
            scope.spawn_producer(|_| async {}).err(),
            Some(ScopeError::Closed)
        );
    }

    #[tokio::test]
    async fn dropping_scope_before_first_poll_releases_captures_without_a_cycle() {
        let registry = ViewScopeRegistry::default();
        let consumer = registry
            .consumer(koushi_protocol::RuntimeConnectionId(3))
            .unwrap();
        let scope = consumer.open().unwrap();
        let control = Arc::downgrade(&scope.control);
        let (held, closed) = oneshot::channel::<()>();
        scope
            .spawn_producer(move |completion| async move {
                let _owned = (held, completion);
                std::future::pending::<()>().await;
            })
            .unwrap();
        let observing_control = control.upgrade().unwrap();
        drop(scope);
        // A short-lived weak-handle upgrade must not keep work running after
        // explicit scope disposal, even though it keeps Control allocated.
        assert!(observing_control.producer.lock().unwrap().is_none());
        assert_eq!(
            *observing_control.retired.lock().unwrap(),
            Some(ViewRetirement::ScopeClosed)
        );
        assert!(
            consumer.open().is_ok(),
            "closing one scope does not retire its consumer"
        );
        drop(observing_control);
        assert!(control.upgrade().is_none());
        assert!(
            tokio::time::timeout(Duration::from_secs(1), closed)
                .await
                .unwrap()
                .is_err()
        );
    }

    #[tokio::test]
    async fn completion_owns_running_lifetime_through_envelope_handling() {
        let registry = ViewScopeRegistry::default();
        let consumer = registry
            .consumer(koushi_protocol::RuntimeConnectionId(2))
            .unwrap();
        let mut scope = consumer.open().unwrap();
        let (sent, received) = oneshot::channel();
        scope
            .spawn_producer(|completion| async move {
                let _ = sent.send(completion);
            })
            .unwrap();
        let completion = received.await.unwrap();
        assert_eq!(
            scope.spawn_producer(|_| async {}).err(),
            Some(ScopeError::ProducerRunning)
        );
        completion.complete();
        scope.spawn_producer(|_completion| async {}).unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), scope.next_delivery())
                .await
                .unwrap(),
            Some(ScopeDelivery::Retired(ViewRetirement::ProducerFailed))
        ));
    }
}
