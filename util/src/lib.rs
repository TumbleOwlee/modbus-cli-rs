pub mod convert;
pub mod tokio;

/// Simple macro to prevent boilerplate of `.to_owned()`
///
/// The macro returns a `String` from the given `&str` value. It removes the boilerplate
/// that normally exists because of various `.to_owned()` calls.
///
/// # Examples
///
/// ```rust
/// use crate::util::str;
///
/// let value: String = str!("Some custom string");
/// ```
#[macro_export]
macro_rules! str {
    ($a:expr) => {
        $a.to_owned()
    };
}

/// Trait providing the `panic()` method that calls the given function and panics with the returned
/// message
///
/// This trait exists to provide the same as `expect()` but with the advantage that you have the
/// error available to include the error into the panic message.
///
/// ```rust
/// #[should_panic]
/// use util::Expect;
///
/// let result: Result<(), &'static str> = Ok(());
/// result.panic(|e| format!("{} just happened", e));
/// ```
pub trait Expect<F: FnOnce(Self::Error) -> String> {
    type Value;
    type Error;

    fn panic(self, f: F) -> Self::Value;
}

/// Generic implementation of Expect for any Result type
impl<T, E, F: FnOnce(E) -> String> Expect<F> for Result<T, E> {
    type Value = T;
    type Error = E;
    fn panic(self, f: F) -> Self::Value {
        match self {
            Ok(v) => v,
            Err(e) => panic!("{}", f(e)),
        }
    }
}

#[macro_export]
macro_rules! async_cloned {
    ($($n:ident),+; $body:block) => (
        {
            $( let $n = $n.clone(); )+
            async move { $body }
        }
    );
}
