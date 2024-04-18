use core::ops::Range;
use core::ptr::addr_of;

use vmm::{
    AddressRangeExt, BumpAllocator, EntryFlags, Flush, FrameAllocator, Mapper, Mode,
    PhysicalAddress, VirtualAddress, INIT,
};

use crate::boot_info::BootInfo;
use crate::elf::ElfSections;
use crate::kconfig;

pub fn init(
    boot_info: &BootInfo,
    kernel: ElfSections,
) -> Result<(usize, VirtualAddress, Range<VirtualAddress>), vmm::Error> {
    // Safety: The boot_info module ensures the memory entries are in the right order
    let alloc: BumpAllocator<INIT<kconfig::MEMORY_MODE>> =
        unsafe { BumpAllocator::new(&boot_info.memories, 0) };

    let mut mapper = Mapper::new(0, alloc)?;
    let mut flush = Flush::empty(0);

    // we're already running in s-mode which means that once we switch on the MMU it takes effect *immediately*
    // as opposed to m-mode where it would take effect after jump tp u-mode.
    // This means we need to temporarily identity map the loader here, so we can continue executing our own code.
    // We will then unmap the loader in the kernel.
    identity_map_self(&mut mapper, &mut flush)?;

    // do the actual mapping
    map_physical_memory(&mut mapper, &mut flush, boot_info)?;
    let fdt_virt = map_fdt(&mut mapper, &mut flush, boot_info)?;
    let stack_virt = map_kernel(&mut mapper, &mut flush, boot_info, kernel)?;

    // Switch on the MMU
    log::debug!("activating page table...");
    let alloc = mapper.activate();

    Ok((alloc.offset(), fdt_virt, stack_virt))
}

fn map_physical_memory(
    mapper: &mut Mapper<INIT<kconfig::MEMORY_MODE>, BumpAllocator<INIT<kconfig::MEMORY_MODE>>>,
    flush: &mut Flush<INIT<kconfig::MEMORY_MODE>>,
    boot_info: &BootInfo,
) -> Result<(), vmm::Error> {
    for region_phys in &boot_info.memories {
        let region_virt = kconfig::MEMORY_MODE::phys_to_virt(region_phys.start)
            ..kconfig::MEMORY_MODE::phys_to_virt(region_phys.end);

        log::trace!("Mapping physical memory region {region_virt:?} => {region_phys:?}...");
        mapper.map_range_with_flush(
            region_virt,
            region_phys.clone(),
            EntryFlags::READ | EntryFlags::WRITE,
            flush,
        )?;
    }

    Ok(())
}

fn map_fdt(
    mapper: &mut Mapper<INIT<kconfig::MEMORY_MODE>, BumpAllocator<INIT<kconfig::MEMORY_MODE>>>,
    flush: &mut Flush<INIT<kconfig::MEMORY_MODE>>,
    boot_info: &BootInfo,
) -> Result<VirtualAddress, vmm::Error> {
    let fdt_phys = unsafe {
        let base = PhysicalAddress::new(boot_info.fdt.as_ptr() as usize);

        (base..base.add(boot_info.fdt.len())).align(kconfig::PAGE_SIZE)
    };
    let fdt_virt = kconfig::MEMORY_MODE::phys_to_virt(fdt_phys.start)
        ..kconfig::MEMORY_MODE::phys_to_virt(fdt_phys.end);

    log::trace!("Mapping fdt region {fdt_virt:?} => {fdt_phys:?}...");
    mapper.map_range_with_flush(fdt_virt, fdt_phys, EntryFlags::READ, flush)?;

    let fdt_addr = unsafe {
        kconfig::MEMORY_MODE::phys_to_virt(PhysicalAddress::new(boot_info.fdt.as_ptr() as usize))
    };

    Ok(fdt_addr)
}

fn identity_map_self(
    mapper: &mut Mapper<INIT<kconfig::MEMORY_MODE>, BumpAllocator<INIT<kconfig::MEMORY_MODE>>>,
    flush: &mut Flush<INIT<kconfig::MEMORY_MODE>>,
) -> Result<(), vmm::Error> {
    extern "C" {
        static __text_start: u8;
        static __text_end: u8;
        static __rodata_start: u8;
        static __rodata_end: u8;
        static __stack_start: u8;
        static __data_end: u8;
    }

    let own_executable_region: Range<PhysicalAddress> = unsafe {
        PhysicalAddress::new(addr_of!(__text_start) as usize)
            ..PhysicalAddress::new(addr_of!(__text_end) as usize)
    };

    let own_read_only_region: Range<PhysicalAddress> = unsafe {
        PhysicalAddress::new(addr_of!(__rodata_start) as usize)
            ..PhysicalAddress::new(addr_of!(__rodata_end) as usize)
    };

    let own_read_write_region: Range<PhysicalAddress> = unsafe {
        PhysicalAddress::new(addr_of!(__stack_start) as usize)
            ..PhysicalAddress::new(addr_of!(__data_end) as usize)
    };

    log::trace!("Identity mapping own executable region {own_executable_region:?}...");
    mapper.identity_map_range_with_flush(
        own_executable_region,
        EntryFlags::READ | EntryFlags::EXECUTE,
        flush,
    )?;

    log::trace!("Identity mapping own read-only region {own_read_only_region:?}...");
    mapper.identity_map_range_with_flush(own_read_only_region, EntryFlags::READ, flush)?;

    log::trace!("Identity mapping own read-write region {own_read_write_region:?}...");
    mapper.identity_map_range_with_flush(
        own_read_write_region,
        EntryFlags::READ | EntryFlags::WRITE,
        flush,
    )?;

    Ok(())
}

fn map_kernel(
    mapper: &mut Mapper<INIT<kconfig::MEMORY_MODE>, BumpAllocator<INIT<kconfig::MEMORY_MODE>>>,
    flush: &mut Flush<INIT<kconfig::MEMORY_MODE>>,
    boot_info: &BootInfo,
    kernel: ElfSections,
) -> Result<Range<VirtualAddress>, vmm::Error> {
    log::trace!(
        "Mapping kernel text region {:?} => {:?}...",
        kernel.text.virt,
        kernel.text.phys
    );
    mapper.map_range_with_flush(
        kernel.text.virt.clone(),
        kernel.text.phys.clone(),
        EntryFlags::READ | EntryFlags::EXECUTE,
        flush,
    )?;

    log::trace!(
        "Mapping kernel rodata region {:?} => {:?}...",
        kernel.rodata.virt,
        kernel.rodata.phys
    );
    mapper.map_range_with_flush(
        kernel.rodata.virt.clone(),
        kernel.rodata.phys.clone(),
        EntryFlags::READ,
        flush,
    )?;

    log::trace!(
        "Mapping kernel bss region {:?} => {:?}...",
        kernel.bss.virt,
        kernel.bss.phys
    );
    mapper.map_range_with_flush(
        kernel.bss.virt.clone(),
        kernel.bss.phys.clone(),
        EntryFlags::READ | EntryFlags::WRITE,
        flush,
    )?;

    log::trace!(
        "Mapping kernel data region {:?} => {:?}...",
        kernel.data.virt,
        kernel.data.phys
    );
    mapper.map_range_with_flush(
        kernel.data.virt.clone(),
        kernel.data.phys.clone(),
        EntryFlags::READ | EntryFlags::WRITE,
        flush,
    )?;

    // Mapping kernel stack region
    let kernel_stack_frames = boot_info.cpus * kconfig::STACK_SIZE_PAGES_KERNEL;

    let kernel_stack_phys = {
        let base = mapper
            .allocator_mut()
            .allocate_frames(kernel_stack_frames)?;
        base..base.add(kernel_stack_frames * kconfig::PAGE_SIZE)
    };

    let kernel_stack_virt = unsafe {
        let end = VirtualAddress::new(kconfig::MEMORY_MODE::PHYS_OFFSET);

        end.sub(kernel_stack_frames * kconfig::PAGE_SIZE)..end
    };

    log::trace!("Mapping kernel stack region {kernel_stack_virt:?} => {kernel_stack_phys:?}...");
    mapper.map_range_with_flush(
        kernel_stack_virt.clone(),
        kernel_stack_phys,
        EntryFlags::READ | EntryFlags::WRITE,
        flush,
    )?;

    Ok(kernel_stack_virt)
}