use stream_guard::{StreamGuardBuilder, StreamEvent};
use futures::{stream, Stream, StreamExt};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn, error, debug};

async fn connect_to_mock_service() -> Result<impl Stream<Item = Result<String, std::io::Error>>, std::io::Error> {
    info!("Establishing connection to mock service...");
    
    // Simulate connection delay
    debug!("Simulating connection delay of 500ms");
    sleep(Duration::from_millis(500)).await;
    debug!("Connection delay completed");
    
    // Create a mock stream that will produce some data then fail
    let stream = stream::iter(vec![
        Ok("Message 1: System initialized".to_string()),
        Ok("Message 2: Processing started".to_string()),
        Ok("Message 3: Data received".to_string()),
        Ok("Message 4: Data received".to_string()),
        Err(std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "Mock connection lost")),
        Ok("Message 6: Data received".to_string()),
        Ok("Message 7: Data received".to_string()),
        Ok("Message 8: Data received".to_string()),
        Ok("Message 9: Data received".to_string()),
        Ok("Message 10: Data received".to_string()),       
        Ok("Message 11: Data received".to_string()),
        Ok("Message 12: Data received".to_string()),
        // Simulate an error that will trigger reconnection
    ]);
    
    Ok(stream)
}

#[tokio::main]
async fn main() {
    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("stream_guard=debug".parse().unwrap()))
        .init();

    info!("StreamGuard Demo - Starting resilient stream monitoring");
    info!("This demo shows automatic reconnection when streams fail");

    let mut stream_guard = StreamGuardBuilder::new(connect_to_mock_service)
        .max_retries(3)
        .next_event_timeout(Duration::from_secs(10))
        .stream_events();

    let mut message_count = 0;
    let max_messages = 10; // Limit demo runtime

    while let Some(event) = stream_guard.next().await {
        match event {
            StreamEvent::Data(data) => {
                message_count += 1;
                info!("📨 Received: {}", data);
                
                if message_count >= max_messages {
                    info!("✅ Demo completed after {} messages", message_count);
                    break;
                }
            },
            StreamEvent::Reconnecting => {
                error!("Connection error detected, stream interrupted");
                warn!("🔄 Connection lost, attempting to reconnect...");
            }
        }
    }

    info!("StreamGuard demo finished");
}