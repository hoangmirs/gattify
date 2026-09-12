//! Orders the commands of one webview against the release of its old page.

use std::{pin::pin, sync::Arc};

use parking_lot::Mutex;
use tokio::sync::Notify;

#[derive(Default)]
struct GateState {
    running: usize,
    cleanups: usize,
}

/// A page load raises a cleanup at once. Commands that arrive during a cleanup
/// wait for it, and the cleanup waits for the commands that already run. So a
/// command of the old page cannot leave a resource behind, and the cleanup
/// cannot close a resource of the new page.
#[derive(Default)]
pub(crate) struct PageGate {
    state: Mutex<GateState>,
    changed: Notify,
}

/// Held by a running command.
pub(crate) struct Pass {
    gate: Arc<PageGate>,
}

impl Drop for Pass {
    fn drop(&mut self) {
        self.gate.state.lock().running -= 1;
        self.gate.changed.notify_waiters();
    }
}

impl PageGate {
    /// Waits until no cleanup is pending, then admits one command.
    pub(crate) async fn enter(self: &Arc<Self>) -> Pass {
        loop {
            let mut changed = pin!(self.changed.notified());
            changed.as_mut().enable();
            {
                let mut state = self.state.lock();
                if state.cleanups == 0 {
                    state.running += 1;
                    return Pass { gate: self.clone() };
                }
            }
            changed.await;
        }
    }

    /// Holds back new commands until the matching [`PageGate::end_cleanup`].
    pub(crate) fn begin_cleanup(&self) {
        self.state.lock().cleanups += 1;
    }

    /// Waits until every admitted command has finished.
    pub(crate) async fn wait_idle(&self) {
        loop {
            let mut changed = pin!(self.changed.notified());
            changed.as_mut().enable();
            if self.state.lock().running == 0 {
                return;
            }
            changed.await;
        }
    }

    pub(crate) fn end_cleanup(&self) {
        {
            let mut state = self.state.lock();
            state.cleanups = state.cleanups.saturating_sub(1);
        }
        self.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_cleanup_waits_for_the_commands_that_already_run() {
        let gate = Arc::new(PageGate::default());
        let pass = gate.enter().await;
        gate.begin_cleanup();
        let idle = tokio::spawn({
            let gate = gate.clone();
            async move { gate.wait_idle().await }
        });
        settle().await;
        assert!(!idle.is_finished());

        drop(pass);
        idle.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_command_waits_for_the_cleanup() {
        let gate = Arc::new(PageGate::default());
        gate.begin_cleanup();
        let command = tokio::spawn({
            let gate = gate.clone();
            async move {
                let _pass = gate.enter().await;
            }
        });
        settle().await;
        assert!(!command.is_finished());

        gate.end_cleanup();
        command.await.unwrap();
    }
}
