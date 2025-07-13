# StreamGuard

A robust Rust library for managing resilient stream connections with automatic reconnection, exponential backoff, and configurable polling strategies.

## Overview

StreamGuard provides a reliable wrapper around async streams that automatically handles connection failures, timeouts, and reconnections. It's designed for scenarios where you need to maintain persistent connections to streaming data sources (APIs, WebSockets, etc.) with built-in fault tolerance.

## Features

- **Automatic Reconnection**: Handles connection failures with configurable retry limits
- **Exponential Backoff**: Smart retry timing to avoid overwhelming failed services
- **Timeout Management**: Configurable timeouts for both connections and stream polling
- **Event Streaming**: Clean abstraction with `StreamEvent` enum for data and reconnection events
- **Flexible Polling Strategies**: Choose between timeout-based or continuous polling
- **Async/Await Support**: Built on Tokio for high-performance async operations

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
stream-guard = "0.1.0"
tokio = { version = "1", features = ["full"] }
futures = "0.3"
```

## Quick Start

Here's a basic example of using StreamGuard to monitor a stream with automatic reconnection:

```rust
use stream_guard::{StreamGuardBuilder, StreamEvent};
use futures::{stream, Stream, StreamExt};
use std::time::Duration;

// Define your connection function
async fn connect_to_service() -> Result<impl Stream<Item = Result<String, std::io::Error>>, std::io::Error> {
    // Your connection logic here - this could be a WebSocket, HTTP stream, etc.
    let stream = stream::iter(vec![
        Ok("data1".to_string()),
        Ok("data2".to_string()),
        Ok("data3".to_string()),
    ]);
    Ok(stream)
}

#[tokio::main]
async fn main() {
    // Create a StreamGuard with custom configuration
    let mut stream_guard = StreamGuardBuilder::new(connect_to_service)
        .max_retries(5)                                    // Retry up to 5 times
        .next_event_timeout(Duration::from_secs(30))       // 30-second timeout per event
        .stream_events();

    // Process events from the guarded stream
    while let Some(event) = stream_guard.next().await {
        match event {
            StreamEvent::Data(data) => {
                println!("Received data: {}", data);
                // Process your data here
            },
            StreamEvent::Reconnecting => {
                println!("Connection lost, attempting to reconnect...");
                // Optional: implement custom reconnection logic
            }
        }
    }
}
```

## Configuration Options

### Builder Pattern

StreamGuard uses a builder pattern for configuration:

```rust
let stream_guard = StreamGuardBuilder::new(your_connect_function)
    .max_retries(10)                                   // Set maximum retry attempts
    .next_event_timeout(Duration::from_secs(60))       // Set per-event timeout
    .stream_events();                                  // Build and start streaming
```

### Polling Strategies

StreamGuard supports two polling strategies:

#### 1. With Timeout (Default)
```rust
let stream_guard = StreamGuardBuilder::new(connect_fn)
    .next_event_timeout(Duration::from_secs(30))  // Timeout after 30 seconds
    .stream_events();
```

#### 2. Without Timeout
```rust
let stream_guard = StreamGuardBuilder::new(connect_fn)
    .without_timeout()  // Wait indefinitely for each event
    .stream_events();
```

## Advanced Examples

### WebSocket Connection with Error Handling

```rust
use stream_guard::{StreamGuardBuilder, StreamEvent};
use futures::StreamExt;
use std::time::Duration;

async fn connect_websocket() -> Result<impl Stream<Item = Result<String, WebSocketError>>, ConnectionError> {
    // WebSocket connection logic
    // This is a placeholder - replace with actual WebSocket implementation
    todo!("Implement WebSocket connection")
}

#[tokio::main]
async fn main() {
    let mut stream = StreamGuardBuilder::new(connect_websocket)
        .max_retries(3)
        .next_event_timeout(Duration::from_secs(45))
        .stream_events();

    while let Some(event) = stream.next().await {
        match event {
            StreamEvent::Data(message) => {
                println!("WebSocket message: {}", message);
            },
            StreamEvent::Reconnecting => {
                println!("WebSocket connection lost, reconnecting...");
            }
        }
    }
}
```

## How It Works

1. **Connection Management**: StreamGuard wraps your connection function and handles initial connection attempts with timeout
2. **Event Polling**: Uses configurable polling strategies to read from the stream
3. **Error Handling**: Automatically detects connection errors, stream errors, and timeouts
4. **Exponential Backoff**: Implements smart retry timing (starts at 2.5s, doubles each attempt, caps at 1 minute)
5. **Event Emission**: Provides clean separation between data events and reconnection events

### Backoff Strategy

The exponential backoff follows this pattern:
- Attempt 1: 2.5 seconds
- Attempt 2: 5 seconds  
- Attempt 3: 10 seconds
- Attempt 4: 20 seconds
- Attempt 5+: 60 seconds (capped)

## Building and Running

### Development

```bash
# Clone the repository
git clone <repository-url>
cd stream-guard

# Build the project
cargo build

# Run tests
cargo test

# Run with example
cargo run 
```

### Testing

The project includes comprehensive tests. Run them with:

```bash
# Run all tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run specific test
cargo test test_stream_guard
```

## Dependencies

- **tokio**: Async runtime with full features
- **futures**: Future and stream utilities
- **thiserror**: Error handling
- **async-trait**: Async traits support
- **async-stream**: Stream generation macros
- **tracing**: Structured logging

## Error Handling

StreamGuard handles several types of errors gracefully:

- **Connection Errors**: Failed initial connections trigger reconnection
- **Stream Errors**: Individual item errors are logged but don't break the stream
- **Timeouts**: Both connection and polling timeouts trigger reconnection
- **Stream End**: Natural stream completion ends the guard loop

