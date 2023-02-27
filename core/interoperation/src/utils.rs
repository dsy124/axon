use ckb_types::core::cell::{CellMeta, CellProvider, CellStatus, ResolvedTransaction};
use ckb_types::core::{DepType, TransactionView};
use ckb_types::{packed, prelude::*};

use protocol::types::InputLock;
use protocol::{lazy::DUMMY_INPUT_OUT_POINT, ProtocolResult};

use crate::InteroperationError;

pub fn resolve_transaction<CL: CellProvider>(
    cell_loader: &CL,
    tx: &TransactionView,
    dummy_input: Option<InputLock>,
) -> ProtocolResult<ResolvedTransaction> {
    let resolve_cell = |out_point: &packed::OutPoint| -> ProtocolResult<CellMeta> {
        match cell_loader.cell(out_point, true) {
            CellStatus::Live(meta) => Ok(meta),
            _ => Err(InteroperationError::GetUnknownCell(out_point.into()).into()),
        }
    };

    let (mut resolved_inputs, mut resolved_cell_deps, mut resolved_dep_groups) = (
        Vec::with_capacity(tx.inputs().len()),
        Vec::with_capacity(tx.cell_deps().len()),
        Vec::with_capacity(tx.cell_deps().len()),
    );

    for outpoint in tx.input_pts_iter() {
        if is_dummy_out_point(&outpoint) {
            if let Some(ref lock) = dummy_input {
                let input_lock = packed::ScriptBuilder::default()
                    .code_hash(lock.lock_code_hash.0.pack())
                    .args(lock.lock_args.pack())
                    .hash_type(lock.hash_type.into())
                    .build();

                resolved_inputs.push(CellMeta {
                    cell_output:        packed::CellOutputBuilder::default()
                        .lock(input_lock)
                        .capacity(lock.capacity().pack())
                        .build(),
                    out_point:          outpoint,
                    transaction_info:   None,
                    data_bytes:         lock.data.len() as u64,
                    mem_cell_data_hash: Some(ckb_hash::blake2b_256(&lock.data).pack()),
                    mem_cell_data:      Some(lock.data.clone()),
                });
            } else {
                return Err(InteroperationError::InvalidDummyInput.into());
            }
        } else {
            resolved_inputs.push(resolve_cell(&outpoint)?);
        }
    }

    for cell_dep in tx.cell_deps_iter() {
        if cell_dep.dep_type() == DepType::DepGroup.into() {
            let dep_group = resolve_cell(&cell_dep.out_point())?;
            let data = dep_group.mem_cell_data.as_ref().unwrap();
            let sub_out_points =
                parse_dep_group_data(data).map_err(InteroperationError::InvalidDepGroup)?;

            for sub_out_point in sub_out_points.into_iter() {
                resolved_cell_deps.push(resolve_cell(&sub_out_point)?);
            }
            resolved_dep_groups.push(dep_group);
        } else {
            resolved_cell_deps.push(resolve_cell(&cell_dep.out_point())?);
        }
    }

    Ok(ResolvedTransaction {
        transaction: tx.clone(),
        resolved_cell_deps,
        resolved_inputs,
        resolved_dep_groups,
    })
}

pub fn parse_dep_group_data(slice: &[u8]) -> Result<packed::OutPointVec, String> {
    if slice.is_empty() {
        Err("data is empty".to_owned())
    } else {
        match packed::OutPointVec::from_slice(slice) {
            Ok(v) => {
                if v.is_empty() {
                    Err("dep group is empty".to_owned())
                } else {
                    Ok(v)
                }
            }
            Err(err) => Err(err.to_string()),
        }
    }
}

pub fn is_dummy_out_point(out_point: &packed::OutPoint) -> bool {
    *out_point == *DUMMY_INPUT_OUT_POINT
}
