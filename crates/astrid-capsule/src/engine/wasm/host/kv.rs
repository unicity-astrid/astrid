use crate::engine::wasm::bindings::astrid::capsule::kv;
use crate::engine::wasm::host::util;
use crate::engine::wasm::host_state::HostState;

impl kv::Host for HostState {
    fn kv_get(&mut self, key: String) -> Result<Option<Vec<u8>>, String> {
        let kv = self.effective_kv().clone();
        util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async {
            kv.get(&key).await
        })
        .map_err(|e| format!("kv_get failed: {e}"))
    }

    fn kv_set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        let kv = self.effective_kv().clone();
        util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async {
            kv.set(&key, value).await
        })
        .map_err(|e| format!("kv_set failed: {e}"))
    }

    fn kv_delete(&mut self, key: String) -> Result<(), String> {
        let kv = self.effective_kv().clone();
        util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async {
            kv.delete(&key).await
        })
        .map(|_| ())
        .map_err(|e| format!("kv_delete failed: {e}"))
    }

    fn kv_list_keys(&mut self, prefix: String) -> Result<Vec<String>, String> {
        let kv = self.effective_kv().clone();
        util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async {
            kv.list_keys_with_prefix(&prefix).await
        })
        .map_err(|e| format!("kv_list_keys failed: {e}"))
    }

    fn kv_clear_prefix(&mut self, prefix: String) -> Result<u64, String> {
        let kv = self.effective_kv().clone();
        let count = util::bounded_block_on(&self.runtime_handle, &self.host_semaphore, async {
            kv.clear_prefix(&prefix).await
        })
        .map_err(|e| format!("kv_clear_prefix failed: {e}"))?;

        Ok(count)
    }
}
