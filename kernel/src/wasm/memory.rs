use crate::wasm::runtime::VMMemoryImport;
use crate::wasm::store::Stored;
use crate::wasm::{Store, runtime};

/// A WebAssembly linear memory instance.
#[derive(Debug, Clone, Copy)]
pub struct Memory(Stored<runtime::ExportedMemory>);

impl Memory {
    // pub fn new(store: &mut Store, ty: MemoryType) -> crate::wasm::Result<Self> {
    //     todo!()
    // }
    // pub fn ty(&self, _store: &Store) -> &MemoryType {
    //     todo!()
    // }
    pub(super) fn as_vmmemory_import(&self, store: &Store) -> VMMemoryImport {
        VMMemoryImport {
            from: store[self.0].definition,
            vmctx: store[self.0].vmctx,
        }
    }

    /// # Safety
    ///
    /// The caller must ensure `export` is a valid exported memory within `store`.
    pub(super) unsafe fn from_vm_export(
        store: &mut Store,
        export: runtime::ExportedMemory,
    ) -> Self {
        Self(store.push_memory(export))
    }

    pub fn comes_from_same_store(self, store: &Store) -> bool {
        store.has_memory(self.0)
    }
}
