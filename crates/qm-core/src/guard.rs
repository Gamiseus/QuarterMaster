//! The panic guard on the DLL path (Phase C D34). Code run through `guarded` has its panics
//! caught, and while it runs `is_guarded` is true on this thread. The DLL's panic hook checks
//! that flag: a guarded panic is only logged (no message box, no abort), and the caller turns
//! mods off for the session instead.
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::any::Any;
use std::cell::Cell;
use std::panic::{self, AssertUnwindSafe};

thread_local! {
    static GUARDED: Cell<bool> = const { Cell::new(false) };
    static LAST_PANIC: Cell<Option<String>> = const { Cell::new(None) };
}

/// True while this thread runs inside `guarded`.
pub fn is_guarded() -> bool {
    GUARDED.with(Cell::get)
}

/// Runs `f` and catches a panic in it. `Err` holds the panic message: the one the panic hook
/// recorded with `record_panic`, else the panic payload. The flag is restored afterwards, so
/// guarded calls nest.
pub fn guarded<R>(f: impl FnOnce() -> R) -> Result<R, String> {
    let prev = GUARDED.with(|g| g.replace(true));
    LAST_PANIC.with(Cell::take);
    let result = panic::catch_unwind(AssertUnwindSafe(f));
    GUARDED.with(|g| g.set(prev));
    result.map_err(|payload| LAST_PANIC.with(Cell::take).unwrap_or_else(|| payload_message(&*payload)))
}

/// Runs `f` with the flag cleared, for the body of every Lua callback (`extern "C"`). A panic
/// cannot unwind out of a callback, so `guarded` could not catch it: with the flag clear the
/// hook writes the crash report and aborts, as before. No destructor holds the old value: a
/// Lua error (longjmp) out of `f` skips the restore, and the flag then stays clear until the
/// enclosing `guarded` returns, so a panic there also reports and aborts.
pub fn unguarded<R>(f: impl FnOnce() -> R) -> R {
    let prev = GUARDED.with(|g| g.replace(false));
    let result = f();
    GUARDED.with(|g| g.set(prev));
    result
}

/// Called by the panic hook for a guarded panic, so `guarded` can report the hook's message
/// (it has the location, the payload does not).
pub fn record_panic(message: String) {
    LAST_PANIC.with(|p| p.set(Some(message)));
}

/// The text of a panic payload.
pub fn payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_is_set_inside_and_restored_after() {
        assert!(!is_guarded());
        assert_eq!(guarded(is_guarded), Ok(true));
        assert!(!is_guarded());
    }

    #[test]
    fn panic_is_caught_with_its_message() {
        let r: Result<(), String> = guarded(|| panic!("boom {}", 7));
        assert_eq!(r, Err("boom 7".to_string()));
        assert!(!is_guarded());
    }

    #[test]
    fn hook_message_wins_and_nesting_keeps_the_flag() {
        let r = guarded(|| {
            let inner: Result<(), String> = guarded(|| {
                record_panic("from the hook".to_string());
                panic!("payload");
            });
            (inner, is_guarded())
        });
        assert_eq!(r, Ok((Err("from the hook".to_string()), true)));
        assert!(!is_guarded());
    }

    #[test]
    fn unguarded_clears_the_flag_and_restores_it() {
        let r = guarded(|| (unguarded(is_guarded), is_guarded()));
        assert_eq!(r, Ok((false, true)));
        assert!(!unguarded(is_guarded));
        assert!(!is_guarded());
    }
}
