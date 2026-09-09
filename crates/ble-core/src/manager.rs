use std::{
    collections::{hash_map::Entry, HashMap},
    sync::atomic::{AtomicU64, Ordering},
};

use parking_lot::Mutex;

use crate::{Backend, BleError, BleResult, Command, ErrorCode, OperationContext, OperationId, OwnerId, Reply};

/// Correlates every operation and attaches owner identity before reaching a backend.
pub struct Manager<B> {
    backend: B,
    next_operation: AtomicU64,
    active_operations: Mutex<HashMap<OperationId, OwnerId>>,
}

impl<B: Backend> Manager<B> {
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            next_operation: AtomicU64::new(1),
            active_operations: Mutex::new(HashMap::new()),
        }
    }

    /// Executes one owner-scoped backend command and attaches its operation ID
    /// to any returned error.
    ///
    /// # Errors
    ///
    /// Returns the structured error produced by the backend.
    pub async fn execute(
        &self,
        owner_id: OwnerId,
        command: Command,
        deadline_millis: Option<u64>,
    ) -> BleResult<Reply> {
        let operation_id = self.allocate_operation_id();
        self.execute_with_id(owner_id, operation_id, command, deadline_millis)
            .await
    }

    /// Executes a command using a caller-generated operation ID.
    ///
    /// This is used at the IPC boundary so an `AbortSignal` can refer to an
    /// in-flight native operation before that operation completes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::InvalidArgument`] for an invalid operation ID,
    /// [`ErrorCode::Busy`] for a duplicate active ID, or the backend error.
    pub async fn execute_with_id(
        &self,
        owner_id: OwnerId,
        operation_id: OperationId,
        command: Command,
        deadline_millis: Option<u64>,
    ) -> BleResult<Reply> {
        if operation_id.as_str().is_empty() || operation_id.as_str().len() > 128 {
            return Err(BleError::new(
                ErrorCode::InvalidArgument,
                "operation ID must contain 1 to 128 bytes",
            ));
        }
        if let Command::Cancel {
            operation_id: target,
        } = &command
        {
            return self.cancel(&owner_id, target).await;
        }
        {
            let mut active = self.active_operations.lock();
            match active.entry(operation_id.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(owner_id.clone());
                }
                Entry::Occupied(_) => {
                    return Err(BleError::new(
                        ErrorCode::Busy,
                        "operation ID is already active",
                    ));
                }
            }
        }
        let _active = ActiveOperation {
            operations: &self.active_operations,
            operation_id: operation_id.clone(),
        };
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

    /// Requests cancellation of an active operation owned by `owner_id`.
    ///
    /// Cancellation is idempotent once an operation has completed. Backends may
    /// be unable to stop an OS procedure immediately, but must ignore and clean
    /// up any stale completion.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::InvalidHandle`] when another owner controls the
    /// target operation, or a structured backend cancellation error.
    pub async fn cancel(
        &self,
        owner_id: &OwnerId,
        operation_id: &OperationId,
    ) -> BleResult<Reply> {
        {
            let active = self.active_operations.lock();
            match active.get(operation_id) {
                None => return Ok(Reply::Empty),
                Some(owner) if owner == owner_id => {}
                Some(_) => return Err(BleError::invalid_handle(operation_id.as_str())),
            }
        }
        let cancel_operation_id = self.allocate_operation_id();
        let context = OperationContext {
            operation_id: cancel_operation_id.clone(),
            owner_id: owner_id.clone(),
            deadline_millis: None,
        };
        self.backend
            .execute(
                context,
                Command::Cancel {
                    operation_id: operation_id.clone(),
                },
            )
            .await
            .map_err(|error| error.with_operation(cancel_operation_id))
    }

    #[must_use]
    pub fn backend(&self) -> &B {
        &self.backend
    }

    fn allocate_operation_id(&self) -> OperationId {
        let sequence = self.next_operation.fetch_add(1, Ordering::Relaxed);
        OperationId::new(format!("op-{sequence}"))
    }
}

struct ActiveOperation<'a> {
    operations: &'a Mutex<HashMap<OperationId, OwnerId>>,
    operation_id: OperationId,
}

impl Drop for ActiveOperation<'_> {
    fn drop(&mut self) {
        self.operations.lock().remove(&self.operation_id);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::poll_fn,
        sync::atomic::{AtomicBool, Ordering},
        task::Poll,
    };

    use async_trait::async_trait;

    use super::*;
    use crate::ScanOptions;

    #[derive(Default)]
    struct CancellableBackend {
        cancelled: AtomicBool,
    }

    #[async_trait]
    impl Backend for CancellableBackend {
        async fn execute(&self, _context: OperationContext, command: Command) -> BleResult<Reply> {
            match command {
                Command::Cancel { .. } => {
                    self.cancelled.store(true, Ordering::Release);
                    Ok(Reply::Empty)
                }
                Command::StartScan(_) => {
                    poll_fn(|context| {
                        if self.cancelled.load(Ordering::Acquire) {
                            Poll::Ready(Err(BleError::new(
                                ErrorCode::Cancelled,
                                "operation was cancelled",
                            )))
                        } else {
                            context.waker().wake_by_ref();
                            Poll::Pending
                        }
                    })
                    .await
                }
                _ => Ok(Reply::Empty),
            }
        }
    }

    #[test]
    fn active_operation_can_be_cancelled_by_its_owner() {
        let manager = Manager::new(CancellableBackend::default());
        let owner = OwnerId::new("owner-a");
        let operation_id = OperationId::new("operation-a");
        futures_lite::future::block_on(async {
            let operation = manager.execute_with_id(
                owner.clone(),
                operation_id.clone(),
                Command::StartScan(ScanOptions {
                    service_uuids: Vec::new(),
                    timeout_ms: None,
                }),
                None,
            );
            let cancellation = async {
                futures_lite::future::yield_now().await;
                manager.cancel(&owner, &operation_id).await
            };
            let (operation, cancellation) = futures_lite::future::zip(operation, cancellation).await;
            assert_eq!(operation.unwrap_err().code, ErrorCode::Cancelled);
            assert_eq!(cancellation.unwrap(), Reply::Empty);
        });
    }

    #[test]
    fn another_owner_cannot_cancel_an_active_operation() {
        let manager = Manager::new(CancellableBackend::default());
        let owner = OwnerId::new("owner-a");
        let operation_id = OperationId::new("operation-a");
        futures_lite::future::block_on(async {
            let operation = manager.execute_with_id(
                owner.clone(),
                operation_id.clone(),
                Command::StartScan(ScanOptions {
                    service_uuids: Vec::new(),
                    timeout_ms: None,
                }),
                None,
            );
            let cancellation = async {
                futures_lite::future::yield_now().await;
                let error = manager
                    .cancel(&OwnerId::new("owner-b"), &operation_id)
                    .await
                    .unwrap_err();
                manager.cancel(&owner, &operation_id).await.unwrap();
                error
            };
            let (operation, cancellation) = futures_lite::future::zip(operation, cancellation).await;
            assert_eq!(operation.unwrap_err().code, ErrorCode::Cancelled);
            assert_eq!(cancellation.code, ErrorCode::InvalidHandle);
        });
    }
}
