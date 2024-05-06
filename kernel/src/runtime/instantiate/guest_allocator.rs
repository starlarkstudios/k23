use crate::frame_alloc::with_frame_alloc;
use crate::kconfig;
use crate::runtime::translate::{MemoryPlan, TablePlan};
use crate::runtime::{VMContext, VMContextOffsets};
use alloc::sync::Arc;
use core::alloc::{AllocError, Allocator, GlobalAlloc, Layout};
use core::fmt;
use core::fmt::Formatter;
use core::ops::Range;
use core::ptr::NonNull;
use linked_list_allocator::LockedHeap;
use vmm::{AddressRangeExt, EntryFlags, Flush, Mapper, Mode, VirtualAddress};

/// A type that knows how to allocate and deallocate memory in userspace.
///
/// We quite often need to allocate things in userspace: compiled module images, stacks,
/// tables, memories, VMContexts, ... essentially everything that is *not* privileged data
/// owned and managed by the kernel is allocated through this.
///
/// This type is used from within a [`Store`] to manage its memory.
///
/// # Address Spaces
///
/// In classical operating systems, a fresh virtual memory address space is allocated to each process
/// for isolation reasons. Conceptually a process owns its allocated address space and backing memory
/// (once a process gets dropped, its resources get freed).
///
/// WebAssembly defines a separate "Data Owner" entity (the [`Store`]) to which all data of one or more
/// instances belongs. So even though a WebAssembly [`Instance`] most closely resembles a process, it is
/// the `Store` that owns the allocated address space and backing memory.
///
/// In k23 we allocate one address space per [`Store`] which turns out to the be same thing as classical
/// operating systems in practice, since each user program gets run in its own [`Store`] .
/// But this approach allows us more flexibility in how we manage memory: E.g. we can have process groups
/// that share a common [`Store`] and can therefore share resources much more efficiently.
#[derive(Debug, Clone)]
pub struct GuestAllocator(Arc<GuestAllocatorInner>);

pub struct GuestAllocatorInner {
    asid: usize,
    root_table: VirtualAddress,
    virt_offset: VirtualAddress,
    // we don't have many allocations, just a few large chunks (e.g. CodeMemory, Stack, Memories)
    // so a simple linked list should suffice.
    // TODO measure and verify this assumption
    inner: LockedHeap,
}

impl GuestAllocator {
    pub unsafe fn new_in_kernel_space(virt_offset: VirtualAddress) -> Self {
        let root_table = kconfig::MEMORY_MODE::get_active_table(0);

        let mut inner = GuestAllocatorInner {
            root_table: kconfig::MEMORY_MODE::phys_to_virt(root_table),
            asid: 0,
            inner: LockedHeap::empty(),
            virt_offset,
        };

        let (mem_virt, flush) = inner.map_additional_pages(32);
        flush.flush().unwrap();

        unsafe {
            inner
                .inner
                .lock()
                .init(mem_virt.start.as_raw() as *mut u8, mem_virt.size());
        }

        Self(Arc::new(inner))
    }

    pub fn asid(&self) -> usize {
        self.0.asid
    }

    pub fn root_table(&self) -> VirtualAddress {
        self.0.root_table
    }

    pub fn allocate_vmctx(&self, offsets: &VMContextOffsets) -> NonNull<VMContext> {
        let vmctx_layout = Layout::from_size_align(offsets.size() as usize, 8).unwrap();
        self.allocate_zeroed(vmctx_layout).unwrap().cast()
    }

    pub fn deallocate_vmctx(&self, ptr: NonNull<VMContext>, offsets: &VMContextOffsets) {
        let vmctx_layout = Layout::from_size_align(offsets.size() as usize, 8).unwrap();
        unsafe { self.deallocate(ptr.cast(), vmctx_layout) };
    }

    pub fn allocate_stack(&self, stack_size: usize) -> NonNull<[u8]> {
        let stack_layout = Layout::from_size_align(stack_size, 16).unwrap();
        self.allocate_zeroed(stack_layout).unwrap()
    }

    pub fn allocate_memory(&self, plan: MemoryPlan) {
        todo!()
    }

    pub fn allocate_table(&self, plan: TablePlan) {
        todo!()
    }
}

unsafe impl Allocator for GuestAllocator {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let ptr = unsafe { self.0.inner.alloc(layout) };

        log::trace!("allocation request {ptr:?} {layout:?}");

        if let Some(ptr) = NonNull::new(ptr) {
            Ok(NonNull::slice_from_raw_parts(ptr, layout.size()))
        } else {
            // TODO map new pages
            // Hitting this case means the inner allocator ran out of memory.
            // we should try to map in new physical memory and grow the alloc

            Err(AllocError)
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        log::trace!("deallocation request {ptr:?} {layout:?}");
        // TODO unmap pages
        self.0.inner.dealloc(ptr.cast().as_ptr(), layout);
    }
}

impl GuestAllocatorInner {
    fn map_additional_pages(
        &mut self,
        num_pages: usize,
    ) -> (Range<VirtualAddress>, Flush<kconfig::MEMORY_MODE>) {
        with_frame_alloc(|frame_alloc| {
            let mut mapper = Mapper::from_address(self.asid, self.root_table, frame_alloc);
            let mut flush = Flush::empty(self.asid);

            let mem_phys = {
                let start = mapper.allocator_mut().allocate_frames(num_pages).unwrap();
                start..start.add(num_pages * kconfig::PAGE_SIZE)
            };

            let mem_virt = self.virt_offset..self.virt_offset.add(num_pages * kconfig::PAGE_SIZE);
            self.virt_offset = mem_virt.end;

            mapper
                .map_range(
                    mem_virt.clone(),
                    mem_phys,
                    EntryFlags::READ | EntryFlags::WRITE,
                    &mut flush,
                )
                .unwrap();

            (mem_virt, flush)
        })
    }
}

impl fmt::Debug for GuestAllocatorInner {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuestAllocatorInner")
            .field("asid", &self.asid)
            .field("root_table", &self.root_table)
            .field("virt_offset", &self.virt_offset)
            .finish()
    }
}