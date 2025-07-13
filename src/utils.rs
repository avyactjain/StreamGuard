use std::time::Duration;

pub fn exponential_backoff(attempt: u32) -> Duration {
    // Use shorter backoff for tests
    #[cfg(test)]
    {
        let base = 100; // 100ms base for tests
        let max_backoff = 1000; // 1 second max for tests
        let backoff_ms = (base * 2_u32.pow(attempt)).min(max_backoff);
        Duration::from_millis(backoff_ms.into())
    }
    
    #[cfg(not(test))]
    {
        // Base duration: 2500ms
        let base = 2500;
        // Cap max backoff of 1 minute
        let max_backoff = 60_000;
        let backoff_ms = (base * 2_u32.pow(attempt)).min(max_backoff);
        Duration::from_millis(backoff_ms.into())
    }
}
