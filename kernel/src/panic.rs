// Copyright 2025 Jonas Kruckenberg
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::arch;
use crate::panic::panic_count::MustAbort;
use alloc::boxed::Box;
use alloc::string::String;
use backtrace::{Backtrace, SymbolizeContext};
use core::any::Any;
use core::panic::{PanicPayload, UnwindSafe};
use core::{fmt, mem, slice};
use loader_api::BootInfo;
use sync::{LazyLock, OnceLock};

static GLOBAL_PANIC_STATE: OnceLock<GlobalPanicState> = OnceLock::new();

struct GlobalPanicState {
    kernel_virt_base: u64,
    elf: &'static [u8],
}

#[cold]
pub fn init(boot_info: &BootInfo) {
    GLOBAL_PANIC_STATE.get_or_init(|| GlobalPanicState {
        kernel_virt_base: boot_info.kernel_virt.start as u64,
        // Safety: we have to trust the loaders BootInfo here
        elf: unsafe {
            let base = boot_info
                .physical_address_offset
                .checked_add(boot_info.kernel_phys.start)
                .unwrap() as *const u8;

            slice::from_raw_parts(
                base,
                boot_info
                    .kernel_phys
                    .end
                    .checked_sub(boot_info.kernel_phys.start)
                    .unwrap(),
            )
        },
    });
}

static SYMBOLIZE_CONTEXT: LazyLock<Option<SymbolizeContext>> = LazyLock::new(|| {
    tracing::debug!("Setting up symbolize context...");
    let state = GLOBAL_PANIC_STATE.get()?;

    let elf = xmas_elf::ElfFile::new(state.elf).unwrap();
    Some(SymbolizeContext::new(elf, state.kernel_virt_base).unwrap())
});

/// Determines whether the current thread is unwinding because of panic.
#[inline]
pub fn panicking() -> bool {
    !panic_count::count_is_zero()
}

/// Invokes a closure, capturing the cause of an unwinding panic if one occurs.
///
/// # Errors
///
/// If the given closure panics, the panic cause will be returned in the Err variant.
pub fn catch_unwind<F, R>(f: F) -> Result<R, Box<dyn Any + Send + 'static>>
where
    F: FnOnce() -> R + UnwindSafe,
{
    #[expect(tail_expr_drop_order, reason = "")]
    unwind2::catch_unwind(f).inspect_err(|_| {
        panic_count::decrease(); // decrease the panic count, since we caught it
    })
}

pub(crate) fn resume_unwind(payload: Box<dyn Any + Send>) {
    rust_panic(payload)
}

/// Entry point for panics from the `core` crate.
#[panic_handler]
fn begin_panic_handler(info: &core::panic::PanicInfo<'_>) -> ! {
    // disable interrupts as soon as we enter the panic subsystem
    // no need to bother with those now as we're about to shut down anyway
    arch::interrupt::disable();

    let loc = info.location().unwrap(); // Currently always returns Some
    let msg = info.message();

    backtrace::__rust_end_short_backtrace(|| {
        if let Some(must_abort) = panic_count::increase(true) {
            match must_abort {
                MustAbort::PanicInHook => {
                    tracing::error!("panicked at {loc}:\n{msg}\n");
                }
            }

            // Run thread-local destructors
            // Safety: after this point we cannot access thread locals anyway
            unsafe {
                cpu_local::destructors::run();
            }

            arch::abort("cpu panicked while processing panic. aborting.");
        }

        tracing::error!("cpu panicked at {loc}:\n{msg}");

        if let Some(ctx) = SYMBOLIZE_CONTEXT.as_ref() {
            // FIXME 32 seems adequate for unoptimized builds where the callstack can get quite deep
            //  but (at least at the moment) is absolute overkill for optimized builds. Sadly there
            //  is no good way to do conditional compilation based on the opt-level.
            const MAX_BACKTRACE_FRAMES: usize = 32;

            let backtrace = Backtrace::<MAX_BACKTRACE_FRAMES>::capture(ctx).unwrap();
            tracing::error!("{backtrace}");
            if backtrace.frames_omitted > 0 {
                let total_frames = backtrace.frames.len() + backtrace.frames_omitted;
                let omitted_frames = backtrace.frames_omitted;

                tracing::warn!("Stack trace was {total_frames} frames, but backtrace buffer capacity was {MAX_BACKTRACE_FRAMES}. Omitted {omitted_frames} frames. Consider increasing `MAX_BACKTRACE_FRAMES` to at least {total_frames} to capture the entire trace.");
            }
        } else {
            tracing::error!(
                "Backtrace unavailable. Panic happened before panic subsystem initialization."
            );
        }

        panic_count::finished_panic_hook();

        if !info.can_unwind() {
            // If a thread panics while running destructors or tries to unwind
            // through a nounwind function (e.g. extern "C") then we cannot continue
            // unwinding and have to abort immediately.
            arch::abort("cpu caused non-unwinding panic. aborting.");
        }

        rust_panic(construct_panic_payload(info))
    })
}

/// Mirroring std, this is an unmangled function on which to slap
/// yer breakpoints for backtracing panics.
#[inline(never)]
#[unsafe(no_mangle)]
fn rust_panic(payload: Box<dyn Any + Send>) -> ! {
    match unwind2::begin_panic(payload) {
        Ok(_) => arch::exit(0),
        Err(unwind2::Error::EndOfStack) => {
            log::error!("unwinding completed without finding a `catch_unwind` make sure there is at least a root level catch unwind wrapping the main function");
            arch::abort("uncaught kernel exception");
        }
        Err(err) => {
            log::error!("unwinding failed with error {err}");
            arch::abort("unwinding failed. aborting.")
        }
    }
}

fn construct_panic_payload(info: &core::panic::PanicInfo) -> Box<dyn Any + Send> {
    struct FormatStringPayload<'a> {
        inner: &'a core::panic::PanicMessage<'a>,
        string: Option<String>,
    }

    impl FormatStringPayload<'_> {
        fn fill(&mut self) -> &mut String {
            let inner = self.inner;
            // Lazily, the first time this gets called, run the actual string formatting.
            self.string.get_or_insert_with(|| {
                let mut s = String::new();
                let mut fmt = fmt::Formatter::new(&mut s);
                let _err = fmt::Display::fmt(&inner, &mut fmt);
                s
            })
        }
    }

    // Safety: TODO
    unsafe impl PanicPayload for FormatStringPayload<'_> {
        fn take_box(&mut self) -> *mut (dyn Any + Send) {
            let contents = mem::take(self.fill());
            Box::into_raw(Box::new(contents))
        }

        fn get(&mut self) -> &(dyn Any + Send) {
            self.fill()
        }
    }

    impl fmt::Display for FormatStringPayload<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            if let Some(s) = &self.string {
                f.write_str(s)
            } else {
                fmt::Display::fmt(&self.inner, f)
            }
        }
    }

    struct StaticStrPayload(&'static str);

    // Safety: TODO
    unsafe impl PanicPayload for StaticStrPayload {
        fn take_box(&mut self) -> *mut (dyn Any + Send) {
            Box::into_raw(Box::new(self.0))
        }

        fn get(&mut self) -> &(dyn Any + Send) {
            &self.0
        }

        fn as_str(&mut self) -> Option<&str> {
            Some(self.0)
        }
    }

    impl fmt::Display for StaticStrPayload {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.0)
        }
    }

    let msg = info.message();
    if let Some(s) = msg.as_str() {
        // Safety: take_box returns an unwrapped box
        unsafe { Box::from_raw(StaticStrPayload(s).take_box()) }
    } else {
        // Safety: take_box returns an unwrapped box
        unsafe {
            Box::from_raw(
                FormatStringPayload {
                    inner: &msg,
                    string: None,
                }
                .take_box(),
            )
        }
    }
}

pub fn payload_as_str(payload: &dyn Any) -> &str {
    if let Some(&s) = payload.downcast_ref::<&'static str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.as_str()
    } else {
        "Box<dyn Any>"
    }
}

mod panic_count {
    use core::{
        cell::Cell,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use cpu_local::cpu_local;

    /// A reason for forcing an immediate abort on panic.
    #[derive(Debug)]
    pub enum MustAbort {
        // AlwaysAbort,
        PanicInHook,
    }

    // Panic count for the current thread and whether a panic hook is currently
    // being executed.
    cpu_local! {
        static LOCAL_PANIC_COUNT: Cell<(usize, bool)> = Cell::new((0, false));
    }

    static GLOBAL_PANIC_COUNT: AtomicUsize = AtomicUsize::new(0);

    pub fn increase(run_panic_hook: bool) -> Option<MustAbort> {
        LOCAL_PANIC_COUNT.with(|c| {
            let (count, in_panic_hook) = c.get();
            if in_panic_hook {
                return Some(MustAbort::PanicInHook);
            }
            c.set((count + 1, run_panic_hook));
            None
        })
    }

    pub fn finished_panic_hook() {
        LOCAL_PANIC_COUNT.with(|c| {
            let (count, _) = c.get();
            c.set((count, false));
        });
    }

    pub fn decrease() {
        GLOBAL_PANIC_COUNT.fetch_sub(1, Ordering::Relaxed);
        LOCAL_PANIC_COUNT.with(|c| {
            let (count, _) = c.get();
            c.set((count - 1, false));
        });
    }

    // Disregards ALWAYS_ABORT_FLAG
    #[must_use]
    #[inline]
    pub fn count_is_zero() -> bool {
        if GLOBAL_PANIC_COUNT.load(Ordering::Relaxed) == 0 {
            // Fast path: if `GLOBAL_PANIC_COUNT` is zero, all threads
            // (including the current one) will have `LOCAL_PANIC_COUNT`
            // equal to zero, so TLS access can be avoided.
            //
            // In terms of performance, a relaxed atomic load is similar to a normal
            // aligned memory read (e.g., a mov instruction in x86), but with some
            // compiler optimization restrictions. On the other hand, a TLS access
            // might require calling a non-inlinable function (such as `__tls_get_addr`
            // when using the GD TLS model).
            true
        } else {
            is_zero_slow_path()
        }
    }

    // Slow path is in a separate function to reduce the amount of code
    // inlined from `count_is_zero`.
    #[inline(never)]
    #[cold]
    fn is_zero_slow_path() -> bool {
        LOCAL_PANIC_COUNT.with(|c| c.get().0 == 0)
    }
}
