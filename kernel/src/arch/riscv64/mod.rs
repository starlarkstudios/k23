// Copyright 2025 Jonas Kruckenberg
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

mod asid_allocator;
pub mod device;
mod setjmp_longjmp;
mod trap_handler;
mod utils;
mod vm;

use crate::device_tree::DeviceTree;
use crate::vm::VirtualAddress;
use crate::wasm;
pub use asid_allocator::AsidAllocator;
use core::arch::asm;
use riscv::sstatus::FS;
use riscv::{interrupt, scounteren, sie, sstatus};
pub use setjmp_longjmp::{JmpBuf, JmpBufStruct, call_with_setjmp, longjmp};
pub use vm::{
    AddressSpace, CANONICAL_ADDRESS_MASK, DEFAULT_ASID, KERNEL_ASPACE_RANGE, PAGE_SHIFT, PAGE_SIZE,
    USER_ASPACE_RANGE, invalidate_range, is_kernel_address,
};

/// Global RISC-V specific initialization.
#[cold]
pub fn init_early() {
    let supported = riscv::sbi::supported_extensions().unwrap();
    tracing::trace!("Supported SBI extensions: {supported:?}");

    vm::init();
    asid_allocator::init();
}

/// Early per-cpu and RISC-V specific initialization.
///
/// This function will be called before global initialization is done, notably this function
/// cannot call logging functions, cannot allocate memory, cannot access cpu-local state and should
/// not panic as the panic handler is not initialized yet.
#[cold]
pub fn per_cpu_init_early() {
    // Safety: register access
    unsafe {
        // enable counters
        scounteren::set_cy();
        scounteren::set_tm();
        scounteren::set_ir();

        // Set the FPU state to initial
        sstatus::set_fs(FS::Initial);
    }
}

/// Late per-cpu and RISC-V specific initialization.
///
/// This function will be called after all global initialization is done.
#[cold]
pub fn per_cpu_init_late(devtree: &DeviceTree) -> crate::Result<()> {
    device::cpu::init(devtree)?;

    // Safety: register access
    unsafe {
        // Initialize the trap handler
        trap_handler::init();

        // Enable interrupts
        interrupt::enable();

        // Enable supervisor timer and external interrupts
        sie::set_ssie();
        sie::set_stie();
        sie::set_seie();
    }

    Ok(())
}

/// Set the thread pointer on the calling cpu to the given address.
pub fn set_thread_ptr(addr: VirtualAddress) {
    // Safety: inline assembly
    unsafe {
        asm!("mv tp, {addr}", addr = in(reg) addr.get());
    }
}

#[inline]
/// Returns the current stack pointer.
pub fn get_stack_pointer() -> usize {
    let stack_pointer: usize;
    // Safety: inline assembly
    unsafe {
        asm!(
            "mv {}, sp",
            out(reg) stack_pointer,
            options(nostack,nomem),
        );
    }
    stack_pointer
}

/// Retrieves the next older program counter and stack pointer from the current frame pointer.
pub unsafe fn get_next_older_pc_from_fp(fp: VirtualAddress) -> VirtualAddress {
    // Safety: caller has to ensure fp is valid
    #[expect(clippy::cast_ptr_alignment, reason = "")]
    unsafe {
        *(fp.as_ptr() as *mut VirtualAddress).offset(1)
    }
}

// The current frame pointer points to the next older frame pointer.
pub const NEXT_OLDER_FP_FROM_FP_OFFSET: usize = 0;

/// Asserts that the frame pointer is sufficiently aligned for the platform.
pub fn assert_fp_is_aligned(fp: VirtualAddress) {
    assert_eq!(fp.get() % 16, 0, "stack should always be aligned to 16");
}

/// Call the WASM array-call trampoline of the provided `func_ref`.
pub unsafe fn array_call(
    func_ref: &wasm::VMFuncRef,
    callee: *mut wasm::VMContext,
    caller: *mut wasm::VMContext,
    args_results_ptr: *mut wasm::VMVal,
    args_results_len: usize,
) {
    // Safety: caller has to ensure safety
    unsafe {
        sstatus::set_spp(sstatus::SPP::User);
        riscv::sepc::set(func_ref.array_call as usize);
        asm! {
            "sret",
            in("a0") callee,
            in("a1") caller,
            in("a2") args_results_ptr,
            in("a3") args_results_len,
            options(noreturn)
        }
    }
}

pub fn mb() {
    // Safety: inline assembly
    unsafe {
        asm!("fence iorw,iorw");
    }
}
pub fn wmb() {
    // Safety: inline assembly
    unsafe {
        asm!("fence ow,ow");
    }
}
pub fn rmb() {
    // Safety: inline assembly
    unsafe {
        asm!("fence ir,ir");
    }
}

/// Suspend the calling cpu indefinitely.
///
/// # Safety
///
/// The caller must ensure it is safe to suspend the cpu.
pub unsafe fn cpu_park() {
    // Safety: inline assembly
    unsafe { asm!("wfi") }
}

/// Send an interrupt to a parked cpu waking it up.
///
/// # Safety
///
/// The caller must ensure it is safe to send an interrupt to the target cpu, which it generally should
/// be as the trap handler for software interrupts should be non-disruptive to already running cpus,
/// but the caller should still exercise caution.
pub unsafe fn cpu_unpark(cpuid: usize) {
    riscv::sbi::ipi::send_ipi(1 << cpuid, 0).unwrap();
}
