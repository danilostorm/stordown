use anyhow::{bail, Result};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::time::sleep;

#[derive(Clone, Default)]
pub struct TransferControl {
    paused: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
}

impl TransferControl {
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Release);
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.paused.store(false, Ordering::Release);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub async fn checkpoint(&self) -> Result<()> {
        while self.is_paused() {
            if self.is_cancelled() {
                bail!("transfer cancelled");
            }

            sleep(Duration::from_millis(100)).await;
        }

        if self.is_cancelled() {
            bail!("transfer cancelled");
        }

        Ok(())
    }
}
