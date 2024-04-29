use crate::wasm::module::Module;
use core::mem;
use core::mem::offset_of;
use core::sync::atomic::AtomicUsize;
use cranelift_wasm::{
    DefinedGlobalIndex, DefinedMemoryIndex, DefinedTableIndex, FuncIndex, GlobalIndex, MemoryIndex,
    OwnedMemoryIndex, TableIndex,
};

// struct VMContext {
//      magic: usize,
//      builtins: *mut VMBuiltinFunctionsArray,
//      store: *mut dyn Store,
//      tables: [VMTableDefinition; module.num_defined_tables],
//      memories: [*mut VMMemoryDefinition; module.num_defined_memories],
//      owned_memories: [VMMemoryDefinition; module.num_owned_memories],
//      globals: [VMGlobalDefinition; module.num_defined_globals],
//      func_refs: [VMFuncRef; module.num_escaped_funcs],
//      imported_functions: [VMFunctionImport; module.num_imported_functions],
//      imported_tables: [VMTableImport; module.num_imported_tables],
//      imported_memories: [VMMemoryImport; module.num_imported_memories],
//      imported_globals: [VMGlobalImport; module.num_imported_globals],
// }

#[derive(Debug)]
#[repr(C)]
pub struct VMBuiltinFunctionsArray {}

#[derive(Debug)]
#[repr(C)]
pub struct VMFunctionImport {}

#[derive(Debug)]
#[repr(C)]
pub struct VMTableImport {
    /// A pointer to the imported table description.
    pub from: *mut VMTableDefinition,
    /// A pointer to the `VMContext` that owns the table description.
    pub vmctx: *mut (),
}

#[derive(Debug)]
#[repr(C)]
pub struct VMMemoryImport {
    /// A pointer to the imported memory description.
    pub from: *mut VMMemoryDefinition,
    /// A pointer to the `VMContext` that owns the memory description.
    pub vmctx: *mut (),
    /// The index of the memory in the containing `vmctx`.
    pub index: DefinedMemoryIndex,
}

#[derive(Debug)]
#[repr(C)]
pub struct VMGlobalImport {
    /// A pointer to the imported global variable description.
    pub from: *mut VMGlobalDefinition,
}

#[derive(Debug)]
#[repr(C)]
pub struct VMTableDefinition {
    /// Pointer to the table data.
    pub base: *mut u8,
    /// The current number of elements in the table.
    pub current_elements: u32,
}

#[derive(Debug)]
#[repr(C)]
pub struct VMMemoryDefinition {
    /// The address space identifier of the memory
    pub asid: usize,
    /// The start address.
    pub base: *mut u8,
    /// The current logical size of this linear memory in bytes.
    ///
    /// This is atomic because shared memories must be able to grow their length
    /// atomically. For relaxed access, see
    /// [`VMMemoryDefinition::current_length()`].
    pub current_length: AtomicUsize,
}

#[derive(Debug)]
#[repr(C)]
pub struct VMGlobalDefinition {
    storage: [u8; 16],
}

pub struct VMContextOffsets {
    num_imported_functions: u32,
    num_imported_tables: u32,
    num_imported_memories: u32,
    num_imported_globals: u32,
    num_defined_tables: u32,
    num_defined_memories: u32,
    num_owned_memories: u32,
    num_defined_globals: u32,
    ptr_size: u32,

    // offsets
    magic: u32,
    builtins_begin: u32,
    tables_begin: u32,
    globals_begin: u32,
    memories_begin: u32,
    owned_memories_begin: u32,
    imported_functions_begin: u32,
    imported_tables_begin: u32,
    imported_memories_begin: u32,
    imported_globals_begin: u32,
}

fn size_of_u32<T: Sized>() -> u32 {
    mem::size_of::<T>() as u32
}

impl VMContextOffsets {
    pub fn new(module: &Module, ptr_size: u32) -> Self {
        let mut offset = 0;

        let mut member_offset = |size_of_field: u32| -> u32 {
            let out = offset;
            offset += size_of_field as u32;
            out
        };

        Self {
            num_imported_functions: module.num_imported_funcs(),
            num_imported_tables: module.num_imported_tables(),
            num_imported_memories: module.num_imported_memories(),
            num_imported_globals: module.num_imported_globals(),
            num_defined_tables: module.num_defined_tables(),
            num_defined_memories: module.num_defined_memories(),
            num_owned_memories: module.num_owned_memories(),
            num_defined_globals: module.num_defined_globals(),
            ptr_size,

            magic: member_offset(size_of_u32::<usize>()),
            builtins_begin: member_offset(ptr_size),
            tables_begin: member_offset(
                size_of_u32::<VMTableDefinition>() * module.num_defined_tables(),
            ),
            globals_begin: member_offset(
                size_of_u32::<VMGlobalDefinition>() * module.num_defined_globals(),
            ),
            memories_begin: member_offset(ptr_size * module.num_defined_memories()),
            owned_memories_begin: member_offset(
                size_of_u32::<VMMemoryDefinition>() * module.num_owned_memories(),
            ),
            imported_functions_begin: member_offset(
                size_of_u32::<VMFunctionImport>() * module.num_imported_funcs(),
            ),
            imported_tables_begin: member_offset(
                size_of_u32::<VMTableImport>() * module.num_imported_tables(),
            ),
            imported_memories_begin: member_offset(
                size_of_u32::<VMMemoryImport>() * module.num_imported_memories(),
            ),
            imported_globals_begin: member_offset(
                size_of_u32::<VMGlobalImport>() * module.num_imported_globals(),
            ),
        }
    }

    #[inline]
    pub fn vmtable_definition(&self, index: DefinedTableIndex) -> u32 {
        assert!(index.as_u32() < self.num_defined_tables);
        self.tables_begin + index.as_u32() * size_of_u32::<VMTableDefinition>()
    }
    #[inline]
    pub fn vmmemory_pointer(&self, index: DefinedMemoryIndex) -> u32 {
        assert!(index.as_u32() < self.num_defined_memories);
        self.memories_begin + index.as_u32() * self.ptr_size
    }
    #[inline]
    pub fn vmmemory_definition(&self, index: OwnedMemoryIndex) -> u32 {
        assert!(index.as_u32() < self.num_owned_memories);
        self.owned_memories_begin + index.as_u32() * size_of_u32::<VMMemoryDefinition>()
    }
    #[inline]
    pub fn vmglobal_definition(&self, index: DefinedGlobalIndex) -> u32 {
        assert!(index.as_u32() < self.num_defined_globals);
        self.globals_begin + index.as_u32() * size_of_u32::<VMGlobalDefinition>()
    }
    #[inline]
    pub fn vmfunction_import(&self, index: FuncIndex) -> u32 {
        assert!(index.as_u32() < self.num_imported_functions);
        self.imported_functions_begin + index.as_u32() * size_of_u32::<VMFunctionImport>()
    }
    #[inline]
    pub fn vmtable_import(&self, index: TableIndex) -> u32 {
        assert!(index.as_u32() < self.imported_tables_begin);
        self.imported_tables_begin + index.as_u32() * size_of_u32::<VMTableImport>()
    }
    #[inline]
    pub fn vmmemory_import(&self, index: MemoryIndex) -> u32 {
        assert!(index.as_u32() < self.num_imported_memories);
        self.imported_memories_begin + index.as_u32() * size_of_u32::<VMMemoryImport>()
    }
    #[inline]
    pub fn vmglobal_import(&self, index: GlobalIndex) -> u32 {
        assert!(index.as_u32() < self.num_imported_globals);
        self.imported_globals_begin + index.as_u32() * size_of_u32::<VMGlobalImport>()
    }

    /// Return the offset to the `base` field in `VMMemoryDefinition` index `index`.
    #[inline]
    pub fn vmmemory_definition_base(&self, index: OwnedMemoryIndex) -> u32 {
        self.vmmemory_definition(index) + offset_of!(VMMemoryDefinition, base) as u32
    }

    /// Return the offset to the `current_length` field in `VMMemoryDefinition` index `index`.
    #[inline]
    pub fn vmmemory_definition_current_length(&self, index: OwnedMemoryIndex) -> u32 {
        self.vmmemory_definition(index) + offset_of!(VMMemoryDefinition, current_length) as u32
    }
}
