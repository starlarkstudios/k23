#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    log::error!("LOADER PANIC {info}");

    kstd::arch::riscv64::abort();
}
