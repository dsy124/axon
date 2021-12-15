#![feature(test)]

use std::collections::BTreeMap;

use evm::executor::stack::{MemoryStackState, StackExecutor, StackSubstateMetadata};

use common_merkle::Merkle;
use protocol::codec::ProtocolCodec;
use protocol::traits::{ApplyBackend, Backend, Executor, ExecutorAdapter as Adapter};
use protocol::types::{
    Account, Config, ExecResp, Hasher, SignedTransaction, TransactionAction, TxResp, H160, H256,
    NIL_DATA, RLP_NULL, U256,
};

pub mod adapter;

#[derive(Default)]
pub struct EvmExecutor;

impl EvmExecutor {
    pub fn new() -> Self {
        EvmExecutor::default()
    }
}

impl Executor for EvmExecutor {
    // Used for query data API, this function will not modify the world state.
    fn call<B: Backend>(&self, backend: &mut B, addr: H160, data: Vec<u8>) -> TxResp {
        let config = Config::london();
        let metadata = StackSubstateMetadata::new(u64::MAX, &config);
        let state = MemoryStackState::new(metadata, backend);
        let precompiles = BTreeMap::new();
        let mut executor = StackExecutor::new_with_precompiles(state, &config, &precompiles);
        let (exit_reason, ret) = executor.transact_call(
            Default::default(),
            addr,
            U256::default(),
            data,
            u64::MAX,
            Vec::new(),
        );

        TxResp {
            exit_reason,
            ret,
            remain_gas: 0,
            gas_used: 0,
            logs: vec![],
        }
    }

    // Function execute returns exit_reason, ret_data and remain_gas.
    fn exec<B: Backend + ApplyBackend + Adapter>(
        &self,
        backend: &mut B,
        txs: Vec<SignedTransaction>,
    ) -> ExecResp {
        let mut res = Vec::new();
        for tx in txs.into_iter() {
            let mut r = self.inner_exec(backend, tx);
            r.logs = backend.get_logs();
            res.push(r);
        }

        ExecResp {
            state_root:   backend.state_root(),
            receipt_root: Merkle::from_hashes(res.iter().map(|r| Hasher::digest(&r.ret)).collect())
                .get_root_hash()
                .unwrap_or_default(),
            gas_used:     res.iter().map(|r| r.gas_used).sum(),
            tx_resp:      res,
        }
    }

    fn get_account<B: Backend + Adapter>(&self, backend: &B, address: &H160) -> Account {
        match backend.get(address.as_bytes()) {
            Some(bytes) => Account::decode(bytes).unwrap(),
            None => Account {
                nonce:        Default::default(),
                balance:      Default::default(),
                storage_root: RLP_NULL,
                code_hash:    NIL_DATA,
            },
        }
    }
}

impl EvmExecutor {
    pub fn inner_exec<B: Backend + ApplyBackend>(
        &self,
        backend: &mut B,
        tx: SignedTransaction,
    ) -> TxResp {
        let config = Config::london();
        let metadata = StackSubstateMetadata::new(u64::MAX, &config);
        let state = MemoryStackState::new(metadata, backend);
        let precompiles = BTreeMap::new();
        let mut executor = StackExecutor::new_with_precompiles(state, &config, &precompiles);
        let (exit_reason, ret) = match tx.transaction.unsigned.action {
            TransactionAction::Call(addr) => executor.transact_call(
                tx.sender,
                addr,
                tx.transaction.unsigned.value,
                tx.transaction.unsigned.input,
                tx.transaction.unsigned.gas_limit.as_u64(),
                tx.transaction
                    .unsigned
                    .access_list
                    .iter()
                    .map(|x| (x.address, x.slots.clone()))
                    .collect(),
            ),
            TransactionAction::Create => {
                let exit_reason = executor.transact_create2(
                    tx.sender,
                    tx.transaction.unsigned.value,
                    tx.transaction.unsigned.input,
                    H256::default(),
                    tx.transaction.unsigned.gas_limit.as_u64(),
                    tx.transaction
                        .unsigned
                        .access_list
                        .iter()
                        .map(|x| (x.address, x.slots.clone()))
                        .collect(),
                );
                (exit_reason, Vec::new())
            }
        };
        let remain_gas = executor.gas();
        let gas_used = executor.used_gas();

        if exit_reason.is_succeed() {
            let (values, logs) = executor.into_state().deconstruct();
            backend.apply(values, logs, true);
        }

        TxResp {
            exit_reason,
            ret,
            remain_gas,
            gas_used,
            logs: vec![],
        }
    }
}
