use std::{fmt, future::Future};

use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct RequestCancellation {
    token: CancellationToken,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestCancelled;

pub type CancellationError = RequestCancelled;
pub type RequestContext = RequestCancellation;

impl RequestCancellation {
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    pub fn from_token(token: CancellationToken) -> Self {
        Self { token }
    }

    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    pub fn cancel(&self) {
        self.token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    pub fn guard(&self) -> RequestCancellationGuard {
        RequestCancellationGuard {
            token: self.token.clone(),
        }
    }

    pub async fn wait<F>(&self, future: F) -> Result<F::Output, RequestCancelled>
    where
        F: Future,
    {
        tokio::select! {
            biased;
            _ = self.token.cancelled() => Err(RequestCancelled),
            output = future => Ok(output),
        }
    }

    pub async fn run_blocking<F, T>(
        &self,
        operation: F,
    ) -> Result<Result<T, tokio::task::JoinError>, RequestCancelled>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        if self.is_cancelled() {
            return Err(RequestCancelled);
        }
        wait_for_blocking(&self.token, tokio::task::spawn_blocking(operation)).await
    }
}

impl Default for RequestCancellation {
    fn default() -> Self {
        Self::new()
    }
}

struct AbortOnDrop<'a, T> {
    handle: &'a mut tokio::task::JoinHandle<T>,
}

impl<'a, T> Drop for AbortOnDrop<'a, T> {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[derive(Clone)]
pub struct RequestCancellationGuard {
    token: CancellationToken,
}

impl RequestCancellationGuard {
    pub fn new(token: CancellationToken) -> Self {
        Self { token }
    }
}

impl Drop for RequestCancellationGuard {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

impl fmt::Display for RequestCancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("request cancelled")
    }
}

impl std::error::Error for RequestCancelled {}

pub async fn wait_for_cancellation<F>(
    token: &CancellationToken,
    future: F,
) -> Result<F::Output, RequestCancelled>
where
    F: Future,
{
    tokio::select! {
        biased;
        _ = token.cancelled() => Err(RequestCancelled),
        output = future => Ok(output),
    }
}

pub async fn wait_for_blocking<T>(
    token: &CancellationToken,
    mut task: tokio::task::JoinHandle<T>,
) -> Result<Result<T, tokio::task::JoinError>, RequestCancelled> {
    let guard = AbortOnDrop {
        handle: &mut task,
    };
    tokio::select! {
        biased;
        _ = token.cancelled() => Err(RequestCancelled),
        result = &mut *guard.handle => Ok(result),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn wait_returns_when_cancelled() {
        let cancellation = RequestCancellation::new();
        let token = cancellation.token();
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            cancellation.wait(async {
                token.cancel();
                tokio::time::sleep(Duration::from_secs(10)).await
            }),
        )
        .await
        .unwrap();
        assert_eq!(result, Err(RequestCancelled));
    }

    #[tokio::test]
    async fn guard_cancels_on_drop() {
        let cancellation = RequestCancellation::new();
        let token = cancellation.token();
        drop(cancellation.guard());
        assert!(token.is_cancelled());
    }
}
