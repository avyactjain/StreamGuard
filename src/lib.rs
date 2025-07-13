use core::pin::pin;
use futures::{Future, Stream};
use std::{fmt::Debug, time::Duration};
use tokio::time::{sleep, timeout};
use tracing::{error, info, warn};

use crate::{
    polling_strategy::{PollingStrategy, WithTimeout, WithoutTimeout},
    utils::exponential_backoff,
};
mod polling_strategy;
mod utils;


#[allow(dead_code)]
struct StreamGuard<ConnectFn, PollingStrategy> {
    pub connect_fn: ConnectFn,
    pub max_retries: u32,
    pub polling_strategy: PollingStrategy,
}

#[derive(Debug)]
pub enum StreamEvent<T> {
    Data(T),
    Reconnecting,
}

#[derive(PartialEq)]
enum StreamAction {
    Reconnect,
    Continue,
    End,
}

pub struct StreamGuardBuilder<ConnectFn, PollingStrategy> {
    connect_fn: ConnectFn,
    max_retries: u32,
    polling_strategy: PollingStrategy,
}

impl<ConnectFn, ConnectErr, T, ItemErr, F, S> StreamGuardBuilder<ConnectFn, WithTimeout>
where
    ConnectFn: FnMut() -> F + Copy,
    F: Future<Output = Result<S, ConnectErr>>,
    S: Stream<Item = Result<T, ItemErr>>,
{
    pub fn new(connect_fn: ConnectFn) -> Self {
        Self {
            connect_fn,
            max_retries: 3,
            polling_strategy: WithTimeout(Duration::from_secs(10)),
        }
    }

    pub fn max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }
}

impl<ConnectFn, ConnectErr, T, ItemErr, F, S, PS> StreamGuardBuilder<ConnectFn, PS>
where
    ConnectFn: FnMut() -> F + Copy,
    F: Future<Output = Result<S, ConnectErr>>,
    S: Stream<Item = Result<T, ItemErr>> + Send,
    PS: PollingStrategy<T, ItemErr>,
    ItemErr: Debug,
    ConnectErr: Debug,
    T: Debug,
{
    pub fn next_event_timeout(&self, t: Duration) -> StreamGuardBuilder<ConnectFn, WithTimeout> {
        StreamGuardBuilder {
            connect_fn: self.connect_fn,
            max_retries: self.max_retries,
            polling_strategy: WithTimeout(t),
        }
    }

    pub fn without_timeout(&self) -> StreamGuardBuilder<ConnectFn, WithoutTimeout> {
        StreamGuardBuilder {
            connect_fn: self.connect_fn,
            max_retries: self.max_retries,
            polling_strategy: WithoutTimeout,
        }
    }

    pub fn stream_events(self) -> impl Stream<Item = StreamEvent<T>> {
        let Self {
            mut connect_fn,
            max_retries,
            polling_strategy,
        } = self;

        // Let's use a default one for now.
        let connect_timeout = Duration::from_secs(1);

        Box::pin(async_stream::stream! {
            let mut try_count = 0;
            loop {
                info!("Attempting to connect (try #{})", try_count + 1);
                let act = match timeout(connect_timeout, connect_fn()).await {
                    Ok(connect_result) => {
                        match connect_result {
                            Ok(stream) => {
                                info!("Successfully connected to stream.");
                                let mut stream = pin!(stream);
                                loop {
                                    let act = match polling_strategy.poll_next(&mut stream).await {
                                        polling_strategy::PollingResult::Item(item) => {
                                            try_count = 0;
                                            info!("Received item from stream.");
                                            yield StreamEvent::Data(item);
                                            continue;
                                        },
                                        polling_strategy::PollingResult::Error(err) => {
                                            warn!("Stream item error: {:?}", err);
                                            StreamAction::Continue
                                        },
                                        polling_strategy::PollingResult::StreamEnded => {
                                            info!("Stream ended.");
                                            StreamAction::End
                                        },
                                        polling_strategy::PollingResult::Timeout => {
                                            warn!("Polling timed out, will attempt to reconnect.");
                                            yield StreamEvent::Reconnecting;
                                            try_count += 1;
                                            StreamAction::Reconnect
                                        },
                                    };

                                    if act != StreamAction::Continue {
                                        break act;
                                    }
                                }
                            },
                            Err(connect_err) => {
                                error!("Connection error: {:?}", connect_err);
                                yield StreamEvent::Reconnecting;
                                try_count += 1;
                                StreamAction::Reconnect
                            },
                        }
                    },
                    Err(_) => {
                        error!("Connection attempt timed out after {:?}", connect_timeout);
                        yield StreamEvent::Reconnecting;
                        try_count += 1;
                        StreamAction::Reconnect
                    },
                };

                if act == StreamAction::End {
                    info!("Exiting stream_events loop: stream ended.");
                    break;
                }

                if try_count <= max_retries {
                    println!("should reconnect now");
                    println!("Trycount is -> {:?}", try_count);

                    let backoff = exponential_backoff(try_count);
                    println!("Sleeping for {:?} before reconnecting", backoff);

                    sleep(backoff).await;
                    // sleep based on exponential backoff.
                    warn!("Reconnecting after failure (try #{})", try_count + 1);
                    continue;

                }
                break
            }
        })
    }
}

#[cfg(test)]
pub mod tests {
    use futures::{stream, StreamExt, Stream};
    use tokio::time::{sleep, timeout, Instant};
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    // Test helper: Simple successful connection
    async fn simple_connect_fn() -> Result<impl Stream<Item = Result<i32, ()>>, ()> {
        let stream = stream::iter(vec![Ok(1), Ok(2), Ok(3), Ok(4), Ok(5)]);
        Ok(stream)
    }

    // Test helper: Connection that fails initially then succeeds
    async fn flaky_connect_fn() -> Result<impl Stream<Item = Result<i32, String>>, String> {
        static CALL_COUNT: AtomicU32 = AtomicU32::new(0);
        let count = CALL_COUNT.fetch_add(1, Ordering::SeqCst);
        
        if count < 2 {
            Err(format!("Connection failed on attempt {}", count + 1))
        } else {
            let stream = stream::iter(vec![Ok(1), Ok(2), Ok(3)]);
            Ok(stream)
        }
    }

    // Test helper: Connection that always fails
    async fn failing_connect_fn() -> Result<impl Stream<Item = Result<i32, ()>>, String> {
        Err::<futures::stream::Empty<Result<i32, ()>>, String>("Connection always fails".to_string())
    }

    // Test helper: Connection that times out during connection
    async fn slow_connect_fn() -> Result<impl Stream<Item = Result<i32, ()>>, ()> {
        sleep(Duration::from_secs(5)).await; // Longer than connection timeout
        let stream = stream::iter(vec![Ok(1), Ok(2), Ok(3)]);
        Ok(stream)
    }

    // Test helper: Stream that produces data slowly
    async fn slow_stream_connect_fn() -> Result<impl Stream<Item = Result<i32, ()>>, ()> {
        let stream = async_stream::stream! {
            yield Ok(1);
            sleep(Duration::from_millis(50)).await;
            yield Ok(2);
            sleep(Duration::from_millis(50)).await;
            yield Ok(3);
        };
        Ok(stream)
    }

    // Test helper: Stream with errors in items
    async fn error_stream_connect_fn() -> Result<impl Stream<Item = Result<i32, String>>, ()> {
        let stream = stream::iter(vec![
            Ok(1),
            Err("Item error".to_string()),
            Ok(2),
            Err("Another error".to_string()),
            Ok(3),
        ]);
        Ok(stream)
    }

    // Test helper: Empty stream
    async fn empty_stream_connect_fn() -> Result<impl Stream<Item = Result<i32, ()>>, ()> {
        let stream = stream::iter(vec![]);
        Ok(stream)
    }

    #[tokio::test]
    async fn test_basic_stream_guard_functionality() {
        let mut stream_guard = StreamGuardBuilder::new(simple_connect_fn)
            .max_retries(2)
            .stream_events();

        let mut received_data = Vec::new();
        let mut event_count = 0;

        while let Some(event) = stream_guard.next().await {
            match event {
                StreamEvent::Data(data) => {
                    received_data.push(data);
                },
                StreamEvent::Reconnecting => {
                    panic!("Should not reconnect for successful stream");
                }
            }
        }

        assert_eq!(received_data, vec![1, 2, 3, 4, 5]);
    }

    #[tokio::test] 
    async fn test_with_timeout_polling_strategy() {
        // Create a stream that will timeout
        let timeout_stream_fn = || async {
            let stream = async_stream::stream! {
                yield Ok::<i32, ()>(1);
                // This delay will cause timeout
                sleep(Duration::from_millis(200)).await;
                yield Ok::<i32, ()>(2);
            };
            Ok::<_, ()>(stream)
        };
        
        let timeout_duration = Duration::from_millis(100);
        
        let mut stream_guard = StreamGuardBuilder::new(timeout_stream_fn)
            .max_retries(1)
            .next_event_timeout(timeout_duration)
            .stream_events();

        let start_time = Instant::now();
        let mut reconnect_count = 0;
        let mut data_count = 0;
        let mut event_count = 0;

        while let Some(event) = stream_guard.next().await {
            event_count += 1;
            match event {
                StreamEvent::Data(data) => {
                    data_count += 1;
                    println!("Received data: {}", data);
                },
                StreamEvent::Reconnecting => {
                    reconnect_count += 1;
                    println!("Reconnecting due to timeout");
                }
            }
            
            // Stop after reasonable time or events to prevent infinite loops
            if start_time.elapsed() > Duration::from_secs(15) || event_count > 10 {
                break;
            }
        }

        // Should receive some data and some reconnection events due to timeouts
        assert!(data_count > 0, "Should receive some data");
        assert!(reconnect_count > 0, "Should have reconnection events due to timeouts");
    }

    #[tokio::test]
    async fn test_without_timeout_polling_strategy() {
        let mut stream_guard = StreamGuardBuilder::new(slow_stream_connect_fn)
            .max_retries(1)
            .without_timeout()
            .stream_events();

        let mut received_data = Vec::new();
        let mut reconnect_count = 0;
        let mut event_count = 0;

        // Use timeout to prevent test from hanging
        let result = timeout(Duration::from_secs(1), async {
            while let Some(event) = stream_guard.next().await {
                event_count += 1;
                match event {
                    StreamEvent::Data(data) => {
                        received_data.push(data);
                    },
                    StreamEvent::Reconnecting => {
                        reconnect_count += 1;
                    }
                }
                
                if event_count > 10 {
                    break;
                }
            }
        }).await;

        // Should complete successfully without timeouts
        assert!(result.is_ok(), "Should complete without timeout");
        assert_eq!(received_data, vec![1, 2, 3]);
        assert_eq!(reconnect_count, 0, "Should not reconnect without timeout strategy");
    }

    #[tokio::test]
    async fn test_connection_failure_and_retry() {
        // Reset the static counter for flaky_connect_fn
        let _ = std::sync::atomic::AtomicU32::new(0);
        
        let mut stream_guard = StreamGuardBuilder::new(flaky_connect_fn)
            .max_retries(5)
            .next_event_timeout(Duration::from_secs(1))
            .stream_events();

        let mut received_data = Vec::new();
        let mut reconnect_count = 0;
        let mut event_count = 0;

        let result = timeout(Duration::from_secs(5), async {
            while let Some(event) = stream_guard.next().await {
                event_count += 1;
                match event {
                    StreamEvent::Data(data) => {
                        received_data.push(data);
                    },
                    StreamEvent::Reconnecting => {
                        reconnect_count += 1;
                    }
                }
                
                if event_count > 15 {
                    break;
                }
            }
        }).await;

        assert!(result.is_ok(), "Test should complete");
        assert!(reconnect_count >= 2, "Should have multiple reconnection attempts");
        assert!(received_data.len() > 0, "Should eventually receive data after retries");
    }

    #[tokio::test]
    async fn test_max_retries_exceeded() {
        let mut stream_guard = StreamGuardBuilder::new(failing_connect_fn)
            .max_retries(2)
            .next_event_timeout(Duration::from_secs(1))
            .stream_events();

        let mut reconnect_count = 0;
        let mut data_count = 0;
        let mut event_count = 0;

        let result = timeout(Duration::from_secs(3), async {
            while let Some(event) = stream_guard.next().await {
                event_count += 1;
                match event {
                    StreamEvent::Data(_) => {
                        data_count += 1;
                    },
                    StreamEvent::Reconnecting => {
                        reconnect_count += 1;
                    }
                }
                
                if event_count > 10 {
                    break;
                }
            }
        }).await;

        assert!(result.is_ok(), "Test should complete when max retries exceeded");
        assert_eq!(data_count, 0, "Should not receive any data when always failing");
        assert!(reconnect_count <= 3, "Should not exceed max retries + 1"); // Initial + retries
    }

    #[tokio::test] 
    async fn test_connection_timeout() {
        let mut stream_guard = StreamGuardBuilder::new(slow_connect_fn)
            .max_retries(2)
            .next_event_timeout(Duration::from_secs(1))
            .stream_events();

        let mut reconnect_count = 0;
        let mut data_count = 0;
        let mut event_count = 0;

        let result = timeout(Duration::from_secs(5), async {
            while let Some(event) = stream_guard.next().await {
                event_count += 1;
                match event {
                    StreamEvent::Data(_) => {
                        data_count += 1;
                    },
                    StreamEvent::Reconnecting => {
                        reconnect_count += 1;
                    }
                }
                
                if event_count > 5 {
                    break;
                }
            }
        }).await;

        assert!(result.is_ok(), "Test should complete");
        assert_eq!(data_count, 0, "Should not receive data when connection times out");
        assert!(reconnect_count > 0, "Should attempt to reconnect after connection timeout");
    }

    #[tokio::test]
    async fn test_stream_item_errors() {
        let mut stream_guard = StreamGuardBuilder::new(error_stream_connect_fn)
            .max_retries(1)
            .next_event_timeout(Duration::from_secs(1))
            .stream_events();

        let mut received_data = Vec::new();
        let mut reconnect_count = 0;
        let mut event_count = 0;

        let result = timeout(Duration::from_secs(2), async {
            while let Some(event) = stream_guard.next().await {
                event_count += 1;
                match event {
                    StreamEvent::Data(data) => {
                        received_data.push(data);
                    },
                    StreamEvent::Reconnecting => {
                        reconnect_count += 1;
                    }
                }
                
                if event_count > 10 {
                    break;
                }
            }
        }).await;

        assert!(result.is_ok(), "Test should complete");
        // Should receive valid data items despite errors
        assert!(received_data.contains(&1), "Should receive first item");
        assert!(received_data.contains(&2), "Should receive second item");
        assert!(received_data.contains(&3), "Should receive third item");
        // Item errors should not cause reconnections
        assert_eq!(reconnect_count, 0, "Item errors should not trigger reconnection");
    }

    #[tokio::test]
    async fn test_empty_stream_ends_gracefully() {
        let mut stream_guard = StreamGuardBuilder::new(empty_stream_connect_fn)
            .max_retries(1)
            .next_event_timeout(Duration::from_secs(1))
            .stream_events();

        let mut received_data = Vec::new();
        let mut reconnect_count = 0;
        let mut event_count = 0;

        let result = timeout(Duration::from_secs(2), async {
            while let Some(event) = stream_guard.next().await {
                event_count += 1;
                match event {
                    StreamEvent::Data(data) => {
                        received_data.push(data);
                    },
                    StreamEvent::Reconnecting => {
                        reconnect_count += 1;
                    }
                }
                
                if event_count > 5 {
                    break;
                }
            }
        }).await;

        assert!(result.is_ok(), "Test should complete");
        assert_eq!(received_data.len(), 0, "Should not receive any data from empty stream");
        assert_eq!(reconnect_count, 0, "Empty stream should end gracefully without reconnection");
    }

    #[tokio::test]
    async fn test_exponential_backoff_timing() {
        use crate::utils::exponential_backoff;
        
        // Test backoff calculation (test values are shorter)
        assert_eq!(exponential_backoff(0), Duration::from_millis(100));
        assert_eq!(exponential_backoff(1), Duration::from_millis(200));
        assert_eq!(exponential_backoff(2), Duration::from_millis(400));
        assert_eq!(exponential_backoff(3), Duration::from_millis(800));
        assert_eq!(exponential_backoff(4), Duration::from_millis(1000)); // Capped at 1 second for tests
        assert_eq!(exponential_backoff(10), Duration::from_millis(1000)); // Still capped
    }

    #[tokio::test]
    async fn test_builder_configuration() {
        let stream_guard = StreamGuardBuilder::new(simple_connect_fn)
            .max_retries(10)
            .next_event_timeout(Duration::from_secs(30));

        // Test that builder configuration is applied
        assert_eq!(stream_guard.max_retries, 10);
        match stream_guard.polling_strategy {
            crate::polling_strategy::WithTimeout(duration) => {
                assert_eq!(duration, Duration::from_secs(30));
            }
            _ => panic!("Expected WithTimeout strategy"),
        }
    }

    #[tokio::test]
    async fn test_without_timeout_configuration() {
        let stream_guard = StreamGuardBuilder::new(simple_connect_fn)
            .max_retries(5)
            .without_timeout();

        // Test that without_timeout configuration is applied
        assert_eq!(stream_guard.max_retries, 5);
        match stream_guard.polling_strategy {
            crate::polling_strategy::WithoutTimeout => {
                // Correct strategy applied
            }
            _ => panic!("Expected WithoutTimeout strategy"),
        }
    }

    // Integration test that combines multiple scenarios
    #[tokio::test]
    async fn test_comprehensive_stream_guard_behavior() {
        // Create a connection function that fails twice, then succeeds with a slow stream
        static CALL_COUNT: AtomicU32 = AtomicU32::new(0);
        
        let complex_connect_fn = || async {
            let count = CALL_COUNT.fetch_add(1, Ordering::SeqCst);
            
            if count < 2 {
                Err(format!("Connection failed on attempt {}", count + 1))
            } else {
                let stream = async_stream::stream! {
                    yield Ok::<i32, String>(1);
                    sleep(Duration::from_millis(200)).await;
                    yield Ok::<i32, String>(2);
                    yield Err::<i32, String>("Stream error".to_string());
                    yield Ok::<i32, String>(3);
                };
                Ok(stream)
            }
        };

        let mut stream_guard = StreamGuardBuilder::new(complex_connect_fn)
            .max_retries(5)
            .next_event_timeout(Duration::from_millis(150))
            .stream_events();

        let mut received_data = Vec::new();
        let mut reconnect_count = 0;
        let mut event_count = 0;

        let result = timeout(Duration::from_secs(15), async {
            while let Some(event) = stream_guard.next().await {
                event_count += 1;
                match event {
                    StreamEvent::Data(data) => {
                        received_data.push(data);
                    },
                    StreamEvent::Reconnecting => {
                        reconnect_count += 1;
                    }
                }
                
                if event_count > 20 {
                    break;
                }
            }
        }).await;

        assert!(result.is_ok(), "Comprehensive test should complete");
        assert!(reconnect_count >= 2, "Should reconnect due to initial failures");
        assert!(received_data.len() > 0, "Should eventually receive some data");
        // Should receive at least the first item before timeout
        assert!(received_data.contains(&1), "Should receive first data item");
    }
}
