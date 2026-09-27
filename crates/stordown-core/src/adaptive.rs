use crate::model::LinkConfig;
use std::{
    cmp::Ordering,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::Mutex, time::sleep};

#[derive(Debug, Clone)]
pub struct AdaptiveLinkPool {
    links: Arc<Vec<LinkConfig>>,
    state: Arc<Mutex<PoolState>>,
}

#[derive(Debug)]
struct PoolState {
    links: Vec<LinkRuntime>,
}

#[derive(Debug, Clone)]
struct LinkRuntime {
    active: usize,
    samples: u64,
    ewma_bps: f64,
    failures: u32,
    cooldown_until: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct LinkLease {
    pub index: usize,
    pub link: LinkConfig,
    started_at: Instant,
}

#[derive(Debug, Clone)]
pub struct LinkHealthSnapshot {
    pub link: LinkConfig,
    pub active: usize,
    pub samples: u64,
    pub ewma_bps: f64,
    pub failures: u32,
    pub cooling_down: bool,
}

impl AdaptiveLinkPool {
    pub fn new(links: Vec<LinkConfig>) -> Self {
        let state = links
            .iter()
            .map(|_| LinkRuntime {
                active: 0,
                samples: 0,
                ewma_bps: 0.0,
                failures: 0,
                cooldown_until: None,
            })
            .collect();

        Self {
            links: Arc::new(links),
            state: Arc::new(Mutex::new(PoolState { links: state })),
        }
    }

    pub async fn acquire(&self) -> LinkLease {
        loop {
            let now = Instant::now();
            let mut state = self.state.lock().await;

            let ready: Vec<usize> = state
                .links
                .iter()
                .enumerate()
                .filter_map(|(index, runtime)| {
                    let ready = runtime
                        .cooldown_until
                        .map(|until| until <= now)
                        .unwrap_or(true);
                    ready.then_some(index)
                })
                .collect();

            if ready.is_empty() {
                let wait = state
                    .links
                    .iter()
                    .filter_map(|runtime| runtime.cooldown_until)
                    .filter(|until| *until > now)
                    .map(|until| until.saturating_duration_since(now))
                    .min()
                    .unwrap_or(Duration::from_millis(250))
                    .min(Duration::from_secs(2));

                drop(state);
                sleep(wait).await;
                continue;
            }

            let index = ready
                .into_iter()
                .max_by(|left, right| {
                    score(&self.links[*left], &state.links[*left])
                        .partial_cmp(&score(&self.links[*right], &state.links[*right]))
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| right.cmp(left))
                })
                .unwrap_or(0);

            state.links[index].active += 1;
            if state.links[index]
                .cooldown_until
                .map(|until| until <= now)
                .unwrap_or(false)
            {
                state.links[index].cooldown_until = None;
            }

            return LinkLease {
                index,
                link: self.links[index].clone(),
                started_at: Instant::now(),
            };
        }
    }

    pub async fn success(&self, lease: &LinkLease, bytes: u64) {
        let elapsed = lease.started_at.elapsed().as_secs_f64().max(0.001);
        let sample_bps = bytes as f64 / elapsed;
        let mut state = self.state.lock().await;
        let runtime = &mut state.links[lease.index];

        runtime.active = runtime.active.saturating_sub(1);
        runtime.samples += 1;
        runtime.failures = runtime.failures.saturating_sub(1);
        runtime.cooldown_until = None;

        if bytes > 0 {
            runtime.ewma_bps = if runtime.ewma_bps <= 0.0 {
                sample_bps
            } else {
                runtime.ewma_bps * 0.72 + sample_bps * 0.28
            };
        }
    }

    pub async fn failure(&self, lease: &LinkLease) {
        let mut state = self.state.lock().await;
        let runtime = &mut state.links[lease.index];

        runtime.active = runtime.active.saturating_sub(1);
        runtime.failures = runtime.failures.saturating_add(1);

        let exponent = runtime.failures.saturating_sub(1).min(5);
        let seconds = 1u64 << exponent;
        runtime.cooldown_until = Some(Instant::now() + Duration::from_secs(seconds.min(30)));
    }

    pub async fn release(&self, lease: &LinkLease) {
        let mut state = self.state.lock().await;
        state.links[lease.index].active = state.links[lease.index].active.saturating_sub(1);
    }

    pub async fn snapshots(&self) -> Vec<LinkHealthSnapshot> {
        let now = Instant::now();
        let state = self.state.lock().await;

        self.links
            .iter()
            .cloned()
            .zip(state.links.iter())
            .map(|(link, runtime)| LinkHealthSnapshot {
                link,
                active: runtime.active,
                samples: runtime.samples,
                ewma_bps: runtime.ewma_bps,
                failures: runtime.failures,
                cooling_down: runtime
                    .cooldown_until
                    .map(|until| until > now)
                    .unwrap_or(false),
            })
            .collect()
    }
}

fn score(link: &LinkConfig, runtime: &LinkRuntime) -> f64 {
    let weight = link.weight.max(1) as f64;
    let active_penalty = runtime.active as f64 + 1.0;
    let failure_penalty = 1.0 + runtime.failures as f64 * 0.75;

    if runtime.samples == 0 || runtime.ewma_bps <= 0.0 {
        return 100_000_000.0 * weight / active_penalty / failure_penalty;
    }

    runtime.ewma_bps * weight / active_penalty / failure_penalty
}

#[cfg(test)]
mod tests {
    use super::AdaptiveLinkPool;
    use crate::model::LinkConfig;
    use std::net::{IpAddr, Ipv4Addr};

    fn link(name: &str, last: u8, weight: u32) -> LinkConfig {
        LinkConfig {
            name: name.to_string(),
            local_ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, last)),
            enabled: true,
            weight,
        }
    }

    #[tokio::test]
    async fn spreads_initial_leases_across_equal_links() {
        let pool = AdaptiveLinkPool::new(vec![link("A", 10, 1), link("B", 11, 1)]);
        let first = pool.acquire().await;
        let second = pool.acquire().await;

        assert_ne!(first.index, second.index);

        pool.release(&first).await;
        pool.release(&second).await;
    }

    #[tokio::test]
    async fn failure_temporarily_moves_work_to_other_link() {
        let pool = AdaptiveLinkPool::new(vec![link("A", 10, 1), link("B", 11, 1)]);
        let failed = pool.acquire().await;
        pool.failure(&failed).await;

        let next = pool.acquire().await;
        assert_ne!(failed.index, next.index);
        pool.release(&next).await;
    }
}
