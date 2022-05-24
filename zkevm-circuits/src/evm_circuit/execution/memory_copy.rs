use crate::{
    evm_circuit::{
        execution::ExecutionGadget,
        param::{N_BYTES_MEMORY_ADDRESS, N_BYTES_MEMORY_WORD_SIZE},
        step::ExecutionState,
        table::TxContextFieldTag,
        util::{
            constraint_builder::{ConstraintBuilder, StepStateTransition, Transition::Delta},
            math_gadget::ComparisonGadget,
            memory_gadget::BufferReaderGadget,
            CachedRegion, Cell,
        },
        witness::{Block, Call, ExecStep, Transaction},
    },
    util::Expr,
};
use bus_mapping::{circuit_input_builder::CopyDetails, constants::MAX_COPY_BYTES};
use eth_types::Field;
use halo2_proofs::plonk::Error;

/// Multi-step gadget for copying data from memory or Tx calldata to memory
#[derive(Clone, Debug)]
pub(crate) struct CopyToMemoryGadget<F> {
    // The src memory address to copy from
    src_addr: Cell<F>,
    // The dst memory address to copy to
    dst_addr: Cell<F>,
    // The number of bytes left to copy
    bytes_left: Cell<F>,
    // The src address bound of the buffer
    src_addr_end: Cell<F>,
    // Indicate whether src is from Tx Calldata
    from_tx: Cell<F>,
    // Source from where we read the bytes. This equals the tx ID in case of a root call, or caller
    // ID in case of an internal call
    src_id: Cell<F>,
    // Buffer reader gadget
    buffer_reader: BufferReaderGadget<F, MAX_COPY_BYTES, N_BYTES_MEMORY_ADDRESS>,
    // The comparison gadget between num bytes copied and bytes_left
    finish_gadget: ComparisonGadget<F, N_BYTES_MEMORY_WORD_SIZE>,
}

impl<F: Field> ExecutionGadget<F> for CopyToMemoryGadget<F> {
    const NAME: &'static str = "COPYTOMEMORY";

    const EXECUTION_STATE: ExecutionState = ExecutionState::CopyToMemory;

    fn configure(cb: &mut ConstraintBuilder<F>) -> Self {
        let src_addr = cb.query_cell();
        let dst_addr = cb.query_cell();
        let bytes_left = cb.query_cell();
        let src_addr_end = cb.query_cell();
        let from_tx = cb.query_bool();
        let src_id = cb.query_cell();
        let buffer_reader = BufferReaderGadget::construct(cb, src_addr.expr(), src_addr_end.expr());
        let from_memory = 1.expr() - from_tx.expr();

        // Copy bytes from src and dst
        for i in 0..MAX_COPY_BYTES {
            let read_flag = buffer_reader.read_flag(i);
            // Read bytes[i] from memory
            cb.condition(from_memory.clone() * read_flag.clone(), |cb| {
                cb.memory_lookup(
                    0.expr(),
                    src_addr.expr() + i.expr(),
                    buffer_reader.byte(i),
                    Some(src_id.expr()),
                )
            });
            // Read bytes[i] from Tx
            cb.condition(from_tx.expr() * read_flag.clone(), |cb| {
                cb.tx_context_lookup(
                    src_id.expr(),
                    TxContextFieldTag::CallData,
                    Some(src_addr.expr() + i.expr()),
                    buffer_reader.byte(i),
                )
            });
            // Write bytes[i] to memory when selectors[i] != 0
            cb.condition(buffer_reader.has_data(i), |cb| {
                cb.memory_lookup(
                    1.expr(),
                    dst_addr.expr() + i.expr(),
                    buffer_reader.byte(i),
                    None,
                )
            });
        }

        let copied_size = buffer_reader.num_bytes();
        let finish_gadget = ComparisonGadget::construct(cb, copied_size.clone(), bytes_left.expr());
        let (lt, finished) = finish_gadget.expr();
        // Constrain lt == 1 or finished == 1
        cb.add_constraint(
            "Constrain num_bytes <= bytes_left",
            (1.expr() - lt) * (1.expr() - finished.clone()),
        );

        // When finished == 0, constraint the CopyToMemory state in next step
        cb.constrain_next_step(
            ExecutionState::CopyToMemory,
            Some(1.expr() - finished),
            |cb| {
                let next_src_addr = cb.query_cell();
                let next_dst_addr = cb.query_cell();
                let next_bytes_left = cb.query_cell();
                let next_src_addr_end = cb.query_cell();
                let next_from_tx = cb.query_cell();
                let next_src_id = cb.query_cell();
                cb.require_equal(
                    "next_src_addr == src_addr + copied_size",
                    next_src_addr.expr(),
                    src_addr.expr() + copied_size.clone(),
                );
                cb.require_equal(
                    "dst_addr + copied_size == next_dst_addr",
                    next_dst_addr.expr(),
                    dst_addr.expr() + copied_size.clone(),
                );
                cb.require_equal(
                    "next_bytes_left == bytes_left - copied_size",
                    next_bytes_left.expr(),
                    bytes_left.expr() - copied_size.clone(),
                );
                cb.require_equal(
                    "next_src_addr_end == src_addr_end",
                    next_src_addr_end.expr(),
                    src_addr_end.expr(),
                );
                cb.require_equal(
                    "next_from_tx == from_tx",
                    next_from_tx.expr(),
                    from_tx.expr(),
                );
                cb.require_equal("next_src_id == src_id", next_src_id.expr(), src_id.expr());
            },
        );

        // State transition
        let step_state_transition = StepStateTransition {
            rw_counter: Delta(cb.rw_counter_offset()),
            ..Default::default()
        };
        cb.require_step_state_transition(step_state_transition);

        Self {
            src_addr,
            dst_addr,
            bytes_left,
            src_addr_end,
            from_tx,
            src_id,
            buffer_reader,
            finish_gadget,
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut CachedRegion<'_, '_, F>,
        offset: usize,
        block: &Block<F>,
        tx: &Transaction,
        call: &Call,
        step: &ExecStep,
    ) -> Result<(), Error> {
        // Read the auxiliary data.
        let aux = if step.aux_data.is_none() {
            // TODO: Handle error correctly returning err
            unreachable!("could not find aux_data for this step")
        } else {
            step.aux_data.unwrap()
        };

        let from_tx = match aux.copy_details() {
            CopyDetails::TxCallData(root_call) => root_call,
            _ => unreachable!("the source has to come from calldata and not code"),
        };

        self.src_addr
            .assign(region, offset, Some(F::from(aux.src_addr())))?;
        self.dst_addr
            .assign(region, offset, Some(F::from(aux.dst_addr())))?;
        self.bytes_left
            .assign(region, offset, Some(F::from(aux.bytes_left())))?;
        self.src_addr_end
            .assign(region, offset, Some(F::from(aux.src_addr_end())))?;
        self.from_tx
            .assign(region, offset, Some(F::from(from_tx as u64)))?;
        let src_id = if call.is_root { tx.id } else { call.caller_id };
        self.src_id
            .assign(region, offset, Some(F::from(src_id as u64)))?;

        // Fill in selectors and bytes
        let mut rw_idx = 0;
        let mut bytes = vec![0u8; MAX_COPY_BYTES];
        let mut selectors = vec![false; MAX_COPY_BYTES];
        for idx in 0..std::cmp::min(aux.bytes_left() as usize, MAX_COPY_BYTES) {
            let src_addr = aux.src_addr() as usize + idx;
            selectors[idx] = true;
            bytes[idx] = if selectors[idx] && src_addr < aux.src_addr_end() as usize {
                if from_tx {
                    tx.call_data[src_addr]
                } else {
                    rw_idx += 1;
                    block.rws[step.rw_indices[rw_idx]].memory_value()
                }
            } else {
                0
            };
            // increase rw_idx for writing back to memory
            rw_idx += 1
        }

        self.buffer_reader.assign(
            region,
            offset,
            aux.src_addr(),
            aux.src_addr_end(),
            &bytes,
            &selectors,
        )?;

        let num_bytes_copied = std::cmp::min(aux.bytes_left(), MAX_COPY_BYTES as u64);
        self.finish_gadget.assign(
            region,
            offset,
            F::from(num_bytes_copied),
            F::from(aux.bytes_left()),
        )?;

        Ok(())
    }
}

#[cfg(test)]
pub mod test {
    use crate::evm_circuit::{
        execution::memory_copy::MAX_COPY_BYTES,
        step::ExecutionState,
        table::RwTableTag,
        test::{rand_bytes, run_test_circuit_incomplete_fixed_table},
        witness::{Block, Bytecode, Call, CodeSource, ExecStep, Rw, RwMap, Transaction},
    };
    use bus_mapping::circuit_input_builder::{CopyDetails, StepAuxiliaryData};
    use eth_types::evm_types::OpcodeId;
    use halo2_proofs::arithmetic::BaseExt;
    use halo2_proofs::pairing::bn256::Fr;
    use std::collections::HashMap;

    pub(crate) const CALL_ID: usize = 1;
    pub(crate) const CALLER_ID: usize = 0;
    pub(crate) const TX_ID: usize = 1;

    #[allow(clippy::too_many_arguments)]
    fn make_memory_copy_step(
        src_addr: u64,
        dst_addr: u64,
        src_addr_end: u64,
        bytes_left: usize,
        from_tx: bool,
        program_counter: u64,
        stack_pointer: usize,
        memory_size: u64,
        rw_counter: usize,
        rws: &mut RwMap,
        bytes_map: &HashMap<u64, u8>,
    ) -> (ExecStep, usize) {
        let mut rw_offset: usize = 0;
        let memory_rws: &mut Vec<_> = rws.0.entry(RwTableTag::Memory).or_insert_with(Vec::new);
        let rw_idx_start = memory_rws.len();
        for idx in 0..std::cmp::min(bytes_left, MAX_COPY_BYTES) {
            let addr = src_addr + idx as u64;
            let byte = if addr < src_addr_end {
                assert!(bytes_map.contains_key(&addr));
                if !from_tx {
                    memory_rws.push(Rw::Memory {
                        rw_counter: rw_counter + rw_offset,
                        is_write: false,
                        call_id: CALLER_ID,
                        memory_address: src_addr + idx as u64,
                        byte: bytes_map[&addr],
                    });
                    rw_offset += 1;
                }
                bytes_map[&addr]
            } else {
                0
            };
            // write to destination memory
            memory_rws.push(Rw::Memory {
                rw_counter: rw_counter + rw_offset,
                is_write: true,
                call_id: CALL_ID,
                memory_address: dst_addr + idx as u64,
                byte,
            });
            rw_offset += 1;
        }
        let rw_idx_end = rws.0[&RwTableTag::Memory].len();
        let aux_data = StepAuxiliaryData::new(
            src_addr,
            dst_addr,
            bytes_left as u64,
            src_addr_end,
            CopyDetails::TxCallData(from_tx),
        );
        let step = ExecStep {
            execution_state: ExecutionState::CopyToMemory,
            rw_indices: (rw_idx_start..rw_idx_end)
                .map(|idx| (RwTableTag::Memory, idx))
                .collect(),
            rw_counter,
            program_counter,
            stack_pointer,
            memory_size,
            gas_cost: 0,
            aux_data: Some(aux_data),
            ..Default::default()
        };
        (step, rw_offset)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn make_memory_copy_steps(
        buffer: &[u8],
        buffer_addr: u64, // buffer base address, use 0 for tx calldata
        src_addr: u64,
        dst_addr: u64,
        length: usize,
        from_tx: bool,
        program_counter: u64,
        stack_pointer: usize,
        memory_size: u64,
        rw_counter: &mut usize,
        rws: &mut RwMap,
        steps: &mut Vec<ExecStep>,
    ) {
        let buffer_addr_end = buffer_addr + buffer.len() as u64;
        let bytes_map = (buffer_addr..buffer_addr_end)
            .zip(buffer.iter().copied())
            .collect();

        let mut copied = 0;
        while copied < length {
            let (step, rw_offset) = make_memory_copy_step(
                src_addr + copied as u64,
                dst_addr + copied as u64,
                buffer_addr_end,
                length - copied,
                from_tx,
                program_counter,
                stack_pointer,
                memory_size,
                *rw_counter,
                rws,
                &bytes_map,
            );
            steps.push(step);
            *rw_counter += rw_offset;
            copied += MAX_COPY_BYTES;
        }
    }

    fn test_ok_from_memory(src_addr: u64, dst_addr: u64, src_addr_end: u64, length: usize) {
        let randomness = Fr::rand();
        let bytecode = Bytecode::new(vec![OpcodeId::STOP.as_u8()]);
        let mut rws = RwMap(Default::default());
        let mut rw_counter = 1;
        let mut steps = Vec::new();
        let buffer = rand_bytes((src_addr_end - src_addr) as usize);
        let memory_size = (dst_addr + length as u64 + 31) / 32 * 32;

        make_memory_copy_steps(
            &buffer,
            src_addr,
            src_addr,
            dst_addr,
            length,
            false,
            0,
            1024,
            memory_size,
            &mut rw_counter,
            &mut rws,
            &mut steps,
        );

        steps.push(ExecStep {
            execution_state: ExecutionState::STOP,
            rw_counter,
            program_counter: 0,
            stack_pointer: 1024,
            memory_size,
            opcode: Some(OpcodeId::STOP),
            ..Default::default()
        });

        let block = Block {
            randomness,
            txs: vec![Transaction {
                id: TX_ID,
                calls: vec![Call {
                    id: CALL_ID,
                    is_root: false,
                    is_create: false,
                    code_source: CodeSource::Account(bytecode.hash),
                    caller_id: CALLER_ID,
                    ..Default::default()
                }],
                steps,
                ..Default::default()
            }],
            rws,
            bytecodes: vec![bytecode],
            ..Default::default()
        };
        assert_eq!(run_test_circuit_incomplete_fixed_table(block), Ok(()));
    }

    fn test_ok_from_tx(calldata_length: usize, src_addr: u64, dst_addr: u64, length: usize) {
        let randomness = Fr::rand();
        let bytecode = Bytecode::new(vec![OpcodeId::STOP.as_u8(), OpcodeId::STOP.as_u8()]);
        let mut rws = RwMap(Default::default());
        let mut rw_counter = 1;
        let calldata: Vec<u8> = rand_bytes(calldata_length);
        let mut steps = Vec::new();
        let memory_size = (dst_addr + length as u64 + 31) / 32 * 32;

        make_memory_copy_steps(
            &calldata,
            0,
            src_addr,
            dst_addr,
            length,
            true,
            0,
            1024,
            memory_size,
            &mut rw_counter,
            &mut rws,
            &mut steps,
        );

        steps.push(ExecStep {
            execution_state: ExecutionState::STOP,
            rw_counter,
            program_counter: 0,
            stack_pointer: 1024,
            memory_size,
            opcode: Some(OpcodeId::STOP),
            ..Default::default()
        });

        let block = Block {
            randomness,
            txs: vec![Transaction {
                id: TX_ID,
                call_data: calldata,
                call_data_length: calldata_length,
                calls: vec![Call {
                    id: CALL_ID,
                    is_root: true,
                    is_create: false,
                    code_source: CodeSource::Account(bytecode.hash),
                    ..Default::default()
                }],
                steps,
                ..Default::default()
            }],
            rws,
            bytecodes: vec![bytecode],
            ..Default::default()
        };
        assert_eq!(run_test_circuit_incomplete_fixed_table(block), Ok(()));
    }

    #[test]
    fn copy_to_memory_simple() {
        test_ok_from_memory(0x40, 0xA0, 0x70, 5);
        test_ok_from_tx(32, 5, 0x40, 5);
    }

    #[test]
    fn copy_to_memory_multi_step() {
        test_ok_from_memory(0x20, 0xA0, 0x80, 80);
        test_ok_from_tx(128, 10, 0x40, 90);
    }

    #[test]
    fn copy_to_memory_out_of_bound() {
        test_ok_from_memory(0x40, 0xA0, 0x60, 45);
        test_ok_from_tx(32, 5, 0x40, 45);
        test_ok_from_tx(32, 40, 0x40, 5);
    }
}
