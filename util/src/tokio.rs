use async_trait::async_trait;
use futures_util::Future;
use once_cell::sync::Lazy;
use std::cell::RefCell;
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
/// A task spawned by `util::tokio::spawn_detach()` or `util::tokio::spawn()` will be part of
/// the global context. These tasks can be joined using `util::tokio::join_all()`.
#[allow(dead_code)]
pub static GLOBAL_CONTEXT: &str = "";

/// Context storage.
///
/// For each named context this structure collects all `JoinHandle` return by `tokio::spawn`
static CONTEXT: Lazy<Mutex<RefCell<Context>>> = Lazy::new(|| Mutex::new(RefCell::default()));

/// Spawn the given future as a tokio task in background in the global context ("")
///
/// The future will be passed to tokio::spawn to create a task. The returned JoinHandle will not be
/// returned to the caller. Instead it will be stored in the static background context. You will
/// have to await the call else the task will not be stored in the background context at all.
/// This context is used to provide the `join_all()` and `join_all_of_context(ctx)` functionality.
/// See the respective documentation for details.
///
/// # Examples
///
/// ```rust
/// use util::tokio::spawn_detach;
///
/// #[tokio::main]
/// async fn main() {
///     // Start the given future as tokio task detached
///     spawn_detach(async move {
///         // do something
///     }).await;
/// }
/// ```
pub async fn spawn_detach<F: Send + IntoFuture + Future + 'static>(future: F)
where
    <F as Future>::Output: Send + Sync + 'static,
{
    let handle: JoinHandle<<F as Future>::Output> = tokio_spawn(future);
    let mut context = CONTEXT.lock().await;
    let collection = &mut context.get_mut().0;
    if let Some(v) = collection.get_mut("") {
        v.push(Box::new(handle));
    } else {
        drop(collection.insert("", vec![Box::new(handle)]));
    }
}

/// Spawn the given future as a tokio task in background in the given context
///
/// The future will be passed to tokio::spawn to create a task. The returned JoinHandle will not be
/// returned to the caller. Instead it will be stored in the named static background context given by `ctx`.
/// You will have to await the call else the task will not be stored in the background context at all.
/// This context is used to provide the `join_all()` and `join_all_of_context(ctx)` functionality.
/// See the respective documentation for details.
///
/// # Examples
///
/// ```rust
/// use util::tokio::spawn_detach_with_context;
///
/// #[tokio::main]
/// async fn main() {
///     // Start the given future as tokio task detached
///     spawn_detach_with_context("Context", async move {
///         // do something
///     }).await;
/// }
/// ```
pub async fn spawn_detach_with_context<F: Send + IntoFuture + Future + 'static>(
    ctx: &'static str,
    future: F,
) where
    <F as Future>::Output: Send + Sync + 'static,
{
    let handle: JoinHandle<<F as Future>::Output> = tokio_spawn(future);
    let mut context = CONTEXT.lock().await;
    let collection = &mut context.get_mut().0;
    if let Some(v) = collection.get_mut(ctx) {
        v.push(Box::new(handle));
    } else {
        drop(collection.insert(ctx, vec![Box::new(handle)]));
    }
}

/// Join all tasks that are stored in any of the contexts
///
/// Each task spawned by `tokio::crate::spawn_detach()` or `tokio::crate::spawn_detach_with_context()`
/// will be part of a background context. A call to `join_all()` will await all stored JoinHandle
/// and will only return if at any given time no more tasks are stored in the context.
///
/// This call will not gurantee that no more tasks are added after returning. It only awaits all
/// tasks that were added before returning.
///
/// # Example
///
/// ```rust
/// use util::tokio::{spawn_detach, spawn_detach_with_context, join_all};
///
/// #[tokio::main]
/// async fn main() {
///     // Start the given future as tokio task detached
///     spawn_detach(async move {
///         println!("Global Context!");
///     }).await;
///
///     spawn_detach_with_context("Local", async move {
///         println!("Local Context!");
///     }).await;
///
///     // Will await both spawned tasks
///     join_all().await;
/// }
/// ```
#[allow(dead_code)]
pub async fn join_all() {
    loop {
        let mut context = CONTEXT.lock().await;
        let handles: Vec<_> = context.get_mut().0.drain().flat_map(|(_, v)| v).collect();
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
/// Each task spawned by `tokio::crate::spawn_detach_with_context()` with the same context name
/// will be awaited by calling `join_all_of_context(ctx)`. A call to `join_all_of_context()` will
/// await all stored JoinHandle and will only return if at any given time no more tasks are stored
/// for the given context name.
///
/// This call will not gurantee that no more tasks are added to the context after returning.
/// It only awaits all tasks that were added before returning.
///
/// # Example
///
/// ```rust
/// use util::tokio::{spawn_detach, spawn_detach_with_context, join_all_of_context};
///
/// #[tokio::main]
/// async fn main() {
///     // Start the given future as tokio task detached
///     spawn_detach(async move {
///         println!("Global Context!");
///     }).await;
///
///     spawn_detach_with_context("Local", async move {
///         println!("Local Context!");
///     }).await;
///
///     // Will only await the JoinHandle added to context "Local"
///     // You will have to call join_all() or join_all(GLOBAL_CONTEXT) to await the other one
///     join_all_of_context("Local").await;
/// }
/// ```
#[allow(dead_code)]
pub async fn join_all_of_context(ctx: &'static str) {
    loop {
        let mut context = CONTEXT.lock().await;
        let handles: Vec<_> = if let Some(v) = context.get_mut().0.get_mut(ctx) {
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
