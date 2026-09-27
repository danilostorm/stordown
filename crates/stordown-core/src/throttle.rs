use std::{sync::Arc, time::Duration};
use tokio::{
    sync::Mutex,
    time::{sleep_until, Instant},
};

#[derive(Debug, Clone)]
pub struct TransferThrottle {
    rate_bytes_per_second: Option<u64>,
    next_slot: Arc<Mutex<Instant>>,
}

impl TransferThrottle {
    pub fn unlimited() -> Self {
        Self::new(None)
    }

    pub fn new(rate_bytes_per_second: Option<u64>) -> Self {
        Self {
            rate_bytes_per_second: rate_bytes_per_second.filter(|value| *value > 0),
            next_slot: Arc::new(Mutex::new(Instant::now())),
        }
    }

    pub fn is_limited(&self) -> bool {
        self.rate_bytes_per_second.is_some()
    }

    pub fn rate_bytes_per_second(&self) -> Option<u64> {
        self.rate_bytes_per_second
    }

    pub async fn consume(&self, bytes: u64) {
        let Some(rate) = self.rate_bytes_per_second else {
            return;
        };

        if bytes == 0 {
            return;
        }

        let now = Instant::now();
        let duration_secs = bytes as f64 / rate as f64;
        let reservation = Duration::from_secs_f64(duration_secs.max(0.000_001));

        let mut next_slot = self.next_slot.lock().await;
        let start = (*next_slot).max(now);
        *next_slot = start + reservation;
        drop(next_slot);

        if start > now {
            sleep_until(start).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TransferThrottle;

    #[tokio::test]
    async fn unlimited_throttle_does_not_enable_limit() {
        let throttle = TransferThrottle::unlimited();
        assert!(!throttle.is_limited());
        assert_eq!(throttle.rate_bytes_per_second(), None);
    }

    #[tokio::test]
    async fn zero_rate_is_treated_as_unlimited() {
        let throttle = TransferThrottle::new(Some(0));
        assert!(!throttle.is_limited());
    }
}
