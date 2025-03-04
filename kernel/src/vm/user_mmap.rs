// Copyright 2025 Jonas Kruckenberg
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

use crate::arch;
use crate::arch::with_user_memory_access;
use crate::traps::TrapMask;
use crate::vm::address::AddressRangeExt;
use crate::vm::{
    AddressSpace, AddressSpaceKind, AddressSpaceRegion, ArchAddressSpace, Batch, Error,
    Permissions, VirtualAddress,
};
use core::alloc::Layout;
use core::num::NonZeroUsize;
use core::range::Range;
use core::slice;

const TRAP_MASK: TrapMask =
    TrapMask::from_bits_retain(TrapMask::StorePageFault.bits() | TrapMask::LoadPageFault.bits());

/// A userspace memory mapping.
///
/// This is essentially a handle to an [`AddressSpaceRegion`] with convenience methods for userspace
/// specific needs such as copying from and to memory.
#[derive(Debug)]
pub struct UserMmap {
    range: Range<VirtualAddress>,
}

// Safety: All mutations of the `*mut AddressSpaceRegion` are happening through a `&mut AddressSpace`
unsafe impl Send for UserMmap {}
// Safety: All mutations of the `*mut AddressSpaceRegion` are happening through a `&mut AddressSpace`
unsafe impl Sync for UserMmap {}

impl UserMmap {
    /// Creates a new empty `Mmap`.
    ///
    /// Note that the size of this cannot be changed after the fact, all accessors will return empty
    /// slices and permission changing methods will always fail.
    pub fn new_empty() -> Self {
        Self {
            range: Range::default(),
        }
    }

    /// Creates a new read-write (`RW`) memory mapping in the given address space.
    pub fn new_zeroed(aspace: &mut AddressSpace, len: usize, align: usize) -> Result<Self, Error> {
        debug_assert!(
            matches!(aspace.kind(), AddressSpaceKind::User),
            "cannot create UserMmap in kernel address space"
        );
        debug_assert!(
            align >= arch::PAGE_SIZE,
            "alignment must be at least a page"
        );

        let layout = Layout::from_size_align(len, align).unwrap();

        let region = aspace.map(
            layout,
            Permissions::READ | Permissions::WRITE | Permissions::USER,
            #[expect(tail_expr_drop_order, reason = "")]
            |range, perms, _batch| Ok(AddressSpaceRegion::new_zeroed(range, perms, None)),
        )?;

        tracing::trace!("new_zeroed: {len} {:?}", region.range);

        Ok(Self {
            range: region.range,
        })
    }

    pub fn range(&self) -> Range<VirtualAddress> {
        self.range
    }

    pub fn copy_from_userspace(
        &self,
        aspace: &mut AddressSpace,
        src_range: Range<usize>,
        dst: &mut [u8],
    ) -> Result<(), Error> {
        self.with_user_slice(aspace, src_range, |src| dst.clone_from_slice(src))
    }

    pub fn copy_to_userspace(
        &mut self,
        aspace: &mut AddressSpace,
        src: &[u8],
        dst_range: Range<usize>,
    ) -> Result<(), Error> {
        self.with_user_slice_mut(aspace, dst_range, |dst| {
            dst.copy_from_slice(src);
        })
    }

    pub fn with_user_slice<F>(
        &self,
        aspace: &mut AddressSpace,
        range: Range<usize>,
        f: F,
    ) -> Result<(), Error>
    where
        F: FnOnce(&[u8]),
    {
        self.commit(aspace, range, false)?;

        #[expect(tail_expr_drop_order, reason = "")]
        crate::traps::catch_traps(TRAP_MASK, || {
            // Safety: checked by caller and `catch_traps`
            unsafe {
                with_user_memory_access(|| {
                    let slice =
                        slice::from_raw_parts(self.range.start.as_ptr(), self.range().size());

                    f(&slice[range]);
                });
            }
        })
        .map_err(Error::Trap)
    }

    pub fn with_user_slice_mut<F>(
        &mut self,
        aspace: &mut AddressSpace,
        range: Range<usize>,
        f: F,
    ) -> Result<(), Error>
    where
        F: FnOnce(&mut [u8]),
    {
        self.commit(aspace, range, true)?;
        // Safety: user aspace also includes kernel mappings in higher half
        unsafe {
            aspace.arch.activate();
        }

        #[expect(tail_expr_drop_order, reason = "")]
        crate::traps::catch_traps(TRAP_MASK, || {
            // Safety: checked by caller and `catch_traps`
            unsafe {
                with_user_memory_access(|| {
                    let slice = slice::from_raw_parts_mut(
                        self.range.start.as_mut_ptr(),
                        self.range().size(),
                    );
                    f(&mut slice[range]);
                });
            }
        })
        .map_err(Error::Trap)
    }

    /// Returns a pointer to the start of the memory mapped by this `Mmap`.
    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.range.start.as_ptr()
    }

    /// Returns a mutable pointer to the start of the memory mapped by this `Mmap`.
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.range.start.as_mut_ptr()
    }

    /// Returns the size in bytes of this memory mapping.
    #[inline]
    pub fn len(&self) -> usize {
        // Safety: the constructor ensures that the NonNull is valid.
        self.range.size()
    }

    /// Whether this is a mapping of zero bytes
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Mark this memory mapping as executable (`RX`) this will by-design make it not-writable too.
    pub fn make_executable(
        &mut self,
        aspace: &mut AddressSpace,
        _branch_protection: bool,
    ) -> Result<(), Error> {
        tracing::trace!("UserMmap::make_executable: {:?}", self.range);
        self.protect(
            aspace,
            Permissions::READ | Permissions::EXECUTE | Permissions::USER,
        )
    }

    /// Mark this memory mapping as read-only (`R`) essentially removing the write permission.
    pub fn make_readonly(&mut self, aspace: &mut AddressSpace) -> Result<(), Error> {
        tracing::trace!("UserMmap::make_readonly: {:?}", self.range);
        self.protect(aspace, Permissions::READ | Permissions::USER)
    }

    fn protect(
        &mut self,
        aspace: &mut AddressSpace,
        new_permissions: Permissions,
    ) -> Result<(), Error> {
        if !self.range.is_empty() {
            let mut cursor = aspace.regions.find_mut(&self.range.start);
            let mut region = cursor.get_mut().unwrap();

            region.permissions = new_permissions;

            let mut flush = aspace.arch.new_flush();
            // Safety: constructors ensure invariants are maintained
            unsafe {
                aspace.arch.update_flags(
                    self.range.start,
                    NonZeroUsize::new(self.range.size()).unwrap(),
                    new_permissions.into(),
                    &mut flush,
                )?;
            };
            flush.flush()?;
        }

        Ok(())
    }

    fn commit(
        &self,
        aspace: &mut AddressSpace,
        range: Range<usize>,
        will_write: bool,
    ) -> Result<(), Error> {
        if !self.range.is_empty() {
            let mut cursor = aspace.regions.find_mut(&self.range.start);

            let src_range = Range {
                start: self.range.start.checked_add(range.start).unwrap(),
                end: self.range.end.checked_add(range.start).unwrap(),
            };

            let mut batch = Batch::new(&mut aspace.arch);
            cursor
                .get_mut()
                .unwrap()
                .commit(&mut batch, src_range, will_write)?;
            batch.flush()?;
        }

        Ok(())
    }
}
