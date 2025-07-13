use std::{pin::Pin, time::Duration};

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use tokio::time::timeout;

#[async_trait]
pub trait PollingStrategy<T, ItemErr> {
    async fn poll_next<S>(&self, stream: &mut Pin<&mut S>) -> PollingResult<T, ItemErr>
    where
        S: Stream<Item = Result<T, ItemErr>> + Send;
}

pub enum PollingResult<T, ItemErr> {
    Item(T),
    Error(ItemErr),
    StreamEnded,
    Timeout,
}

pub struct WithTimeout(pub Duration);
pub struct WithoutTimeout;

#[async_trait]
impl<T, ItemErr> PollingStrategy<T, ItemErr> for WithTimeout {
    async fn poll_next<S>(&self, stream: &mut Pin<&mut S>) -> PollingResult<T, ItemErr>
    where
        S: Stream<Item = Result<T, ItemErr>> + Send,
    {
        match timeout(self.0, stream.next()).await {
            Ok(Some(Ok(item))) => PollingResult::Item(item),
            Ok(Some(Err(err))) => PollingResult::Error(err),
            Ok(None) => PollingResult::StreamEnded,
            Err(_) => {
                println!("Stream timeout.");
                PollingResult::Timeout
            }
        }
    }
}

#[async_trait]
impl<T, ItemErr> PollingStrategy<T, ItemErr> for WithoutTimeout {
    async fn poll_next<S>(&self, stream: &mut Pin<&mut S>) -> PollingResult<T, ItemErr>
    where
        S: Stream<Item = Result<T, ItemErr>> + Send,
    {
        match stream.next().await {
            Some(Ok(item)) => PollingResult::Item(item),
            Some(Err(err)) => PollingResult::Error(err),
            None => PollingResult::StreamEnded,
        }
    }
}
