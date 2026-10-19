//! Per-uid token bucket.

use std::collections::HashMap;

pub struct RateLimiter {
    rate_per_sec: f64,
    burst: f64,
    buckets: HashMap<u32, (f64, u64)>,
}

impl RateLimiter {
    pub fn new(rate_per_sec: u32, burst: u32) -> Self {
        Self {
            rate_per_sec: rate_per_sec as f64,
            burst: burst as f64,
            buckets: HashMap::new(),
        }
    }

    pub fn allow(&mut self, uid: u32, now_ms: u64) -> bool {
        if self.buckets.len() > 4096 {
            // Bound memory: forget full (idle) buckets.
            let (rate, burst) = (self.rate_per_sec, self.burst);
            self.buckets.retain(|_, (tokens, at)| {
                *tokens + (now_ms.saturating_sub(*at) as f64 / 1000.0) * rate < burst
            });
        }
        let (tokens, at) = self.buckets.entry(uid).or_insert((self.burst, now_ms));
        *tokens = (*tokens + (now_ms.saturating_sub(*at) as f64 / 1000.0) * self.rate_per_sec)
            .min(self.burst);
        *at = now_ms;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn bucket() {
        let mut r = super::RateLimiter::new(2, 3);
        assert!((0..3).all(|_| r.allow(1, 0)));
        assert!(!r.allow(1, 0));
        assert!(r.allow(2, 0), "per uid");
        assert!(r.allow(1, 500));
        assert!(!r.allow(1, 500));
    }
}
