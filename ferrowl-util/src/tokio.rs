use async_trait::async_trait;
use futures_util::Future;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::future::IntoFuture;
use tokio::spawn as tokio_spawn;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

#[async_trait]
trait Joinable: Sync + Send {
    async fn join(&mut self);
}

#[async_trait]
impl<Output> Joinable for JoinHandle<Output>
where
    Output: Send + Sync + 'static,
{
    async fn join(&mut self) {
        if !self.is_finished() {
            drop(self.await);
        }
    }
}

#[derive(Default)]
struct Context(HashMap<&'static str, Vec<Box<dyn Joinable>>>);

/// Name of the global context
///
/// A task spawned by `ferrowl_util::tokio::spawn_detach()` will be part of
/// the global context. These tasks can be joined using `ferrowl_util::tokio::join_all()`.
pub static GLOBAL_CONTEXT: &str = "";

/// For each named context this structure collects all `JoinHandle` return by `tokio::spawn`
static CONTEXT: Lazy<Mutex<Context>> = Lazy::new(|| Mutex::new(Context::default()));

/// Spawn the given future as a tokio task in background in the global context ("")
///
/// The returned `JoinHandle` is not returned to the caller; it is stored in the static
/// background context instead. The call must be awaited, or the task is not stored at all.
///
/// # Examples
///
/// ```rust
/// use ferrowl_util::tokio::spawn_detach;
///
/// #[tokio::main]
/// async fn main() {
///     spawn_detach(async move {}).await;
/// }
/// ```
pub async fn spawn_detach<F: Send + IntoFuture + Future + 'static>(future: F)
where
    <F as Future>::Output: Send + Sync + 'static,
{
    spawn_detach_with_context(GLOBAL_CONTEXT, future).await;
}

/// Spawn the given future as a tokio task in background in the given context
///
/// The returned `JoinHandle` is not returned to the caller; it is stored in the named static
/// background context given by `ctx`. The call must be awaited, or the task is not stored at all.
pub async fn spawn_detach_with_context<F: Send + IntoFuture + Future + 'static>(
    ctx: &'static str,
    future: F,
) where
    <F as Future>::Output: Send + Sync + 'static,
{
    let handle: JoinHandle<<F as Future>::Output> = tokio_spawn(future);
    let mut context = CONTEXT.lock().await;
    context.0.entry(ctx).or_default().push(Box::new(handle));
}

/// Join all tasks that are stored in any of the contexts
///
/// Returns once all handles stored at the time of the call have finished; gives no guarantee
/// that no tasks are added after the call returns.
pub async fn join_all() {
    loop {
        let mut context = CONTEXT.lock().await;
        let handles: Vec<_> = context.0.drain().flat_map(|(_, v)| v).collect();
        drop(context);

        if handles.is_empty() {
            break;
        }

        for mut handle in handles {
            handle.join().await;
        }
    }
}

/// Join all tasks that are stored in the named context
///
/// Returns once all handles stored under `ctx` at the time of the call have finished; gives no
/// guarantee that no tasks are added after the call returns.
pub async fn join_all_of_context(ctx: &'static str) {
    loop {
        let mut context = CONTEXT.lock().await;
        let handles: Vec<_> = if let Some(v) = context.0.get_mut(ctx) {
            std::mem::take(v)
        } else {
            vec![]
        };
        drop(context);

        if handles.is_empty() {
            break;
        }

        for mut handle in handles {
            handle.join().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{join_all, join_all_of_context, spawn_detach, spawn_detach_with_context};
    use std::time::Duration;

    // All spawn/join coverage lives in one test: the background context is a
    // process-global static, so running these assertions concurrently across
    // tests would let one drain another's handles.
    #[tokio::test]
    async fn ut_spawn_and_join_contexts() {
        // Global context: first spawn inserts the vec, second pushes onto it.
        spawn_detach(async {}).await;
        spawn_detach(async {}).await;
        // A still-running task exercises the `!is_finished` await branch of join.
        spawn_detach(async {
            tokio::time::sleep(Duration::from_millis(20)).await;
        })
        .await;

        // Named context: same insert-then-push pattern.
        spawn_detach_with_context("ctxA", async {}).await;
        spawn_detach_with_context("ctxA", async {}).await;

        // Join the named context only.
        join_all_of_context("ctxA").await;
        // Unknown context: nothing stored -> empty -> immediate break.
        join_all_of_context("unknown").await;

        // Join everything remaining in the global context.
        join_all().await;
        // Nothing left -> immediate break.
        join_all().await;
    }
}
