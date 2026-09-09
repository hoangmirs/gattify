use std::sync::atomic::{AtomicU64, Ordering};

use crate::{Backend, BleResult, Command, OperationContext, OperationId, OwnerId, Reply};

/// Correlates every operation and attaches owner identity before reaching a backend.
pub struct Manager<B> {
    backend: B,
    next_operation: AtomicU64,
}

impl<B: Backend> Manager<B> {
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            next_operation: AtomicU64::new(1),
        }
    }

    pub async fn execute(
        &self,
        owner_id: OwnerId,
        command: Command,
        deadline_millis: Option<u64>,
    ) -> BleResult<Reply> {
        let sequence = self.next_operation.fetch_add(1, Ordering::Relaxed);
        let operation_id = OperationId::new(format!("op-{sequence}"));
        let context = OperationContext {
            operation_id: operation_id.clone(),
            owner_id,
            deadline_millis,
        };
        self.backend
            .execute(context, command)
            .await
            .map_err(|error| error.with_operation(operation_id))
    }

    #[must_use]
    pub fn backend(&self) -> &B {
        &self.backend
    }
}

