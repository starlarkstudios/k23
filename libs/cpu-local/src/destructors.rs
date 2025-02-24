// Copyright 2025 Jonas Kruckenberg
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::abort;
use alloc::vec::Vec;
use core::cell::RefCell;

#[thread_local]
static DTORS: RefCell<Vec<(*mut u8, unsafe extern "C" fn(*mut u8))>> = RefCell::new(Vec::new());

pub(crate) unsafe fn register(t: *mut u8, dtor: unsafe extern "C" fn(*mut u8)) {
    let Ok(mut dtors) = DTORS.try_borrow_mut() else {
        // This point can only be reached if the global allocator calls this
        // function again.
        abort("the global allocator may not use TLS with destructors")
    };

    dtors.push((t, dtor));
}

/// Run thread-local destructors
///
/// # Safety
///
/// May only be run on thread exit to guarantee that there are no live references
/// to TLS variables while they are destroyed.
pub unsafe fn run() {
    loop {
        let mut dtors = DTORS.borrow_mut();
        match dtors.pop() {
            Some((t, dtor)) => {
                drop(dtors);

                // Safety: The crates API ensured the memory cannot be accessed after destructors ran
                // plus this module ensures dtors will be run only once
                unsafe {
                    dtor(t);
                }
            }
            None => {
                // Free the list memory.
                *dtors = Vec::new();
                break;
            }
        }
    }
}
