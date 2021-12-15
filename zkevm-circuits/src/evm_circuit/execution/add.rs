use crate::{
    evm_circuit::{
        execution::{
            bus_mapping_tmp::{Block, Call, ExecStep, Transaction},
            ExecutionGadget,
        },
        step::ExecutionState,
        util::{
            common_gadget::SameContextGadget,
            constraint_builder::{
                ConstraintBuilder, StepStateTransition, Transition::Delta,
            },
            math_gadget::{AddWordsGadget, PairSelectGadget},
            select,
        },
    },
    util::Expr,
};
use bus_mapping::evm::OpcodeId;
use halo2::{arithmetic::FieldExt, circuit::Region, plonk::Error};

// AddGadget verifies ADD and SUB at the same time by an extra swap flag,
// when it's ADD, we annotate stack as [a, b, ...] and [c, ...],
// when it's SUB, we annotate stack as [c, b, ...] and [a, ...].
// Then we verify if a + b is equal to c.
#[derive(Clone, Debug)]
pub(crate) struct AddGadget<F> {
    same_context: SameContextGadget<F>,
    add_words: AddWordsGadget<F, 2>,
    is_sub: PairSelectGadget<F>,
}

impl<F: FieldExt> ExecutionGadget<F> for AddGadget<F> {
    const NAME: &'static str = "ADD";

    const EXECUTION_STATE: ExecutionState = ExecutionState::ADD;

    fn configure(cb: &mut ConstraintBuilder<F>) -> Self {
        let opcode = cb.query_cell();

        let a = cb.query_word();
        let b = cb.query_word();
        let add_words = AddWordsGadget::construct(cb, [a.clone(), b.clone()]);
        let c = add_words.sum();

        // Swap a and c if opcode is SUB
        let is_sub = PairSelectGadget::construct(
            cb,
            opcode.expr(),
            OpcodeId::SUB.expr(),
            OpcodeId::ADD.expr(),
        );

        // ADD: Pop a and b from the stack, push c on the stack
        // SUB: Pop c and b from the stack, push a on the stack
        cb.stack_pop(select::expr(is_sub.expr().0, c.expr(), a.expr()));
        cb.stack_pop(b.expr());
        cb.stack_push(select::expr(is_sub.expr().0, a.expr(), c.expr()));

        // State transition
        let step_state_transition = StepStateTransition {
            rw_counter: Delta(3.expr()),
            program_counter: Delta(1.expr()),
            stack_pointer: Delta(1.expr()),
            ..Default::default()
        };
        let same_context = SameContextGadget::construct(
            cb,
            opcode,
            step_state_transition,
            None,
        );

        Self {
            same_context,
            add_words,
            is_sub,
        }
    }

    fn assign_exec_step(
        &self,
        region: &mut Region<'_, F>,
        offset: usize,
        block: &Block<F>,
        _: &Transaction<F>,
        _: &Call<F>,
        step: &ExecStep,
    ) -> Result<(), Error> {
        self.same_context.assign_exec_step(region, offset, step)?;

        let opcode = step.opcode.unwrap();
        let indices = if opcode == OpcodeId::SUB {
            [step.rw_indices[2], step.rw_indices[1], step.rw_indices[0]]
        } else {
            [step.rw_indices[0], step.rw_indices[1], step.rw_indices[2]]
        };
        let [a, b, c] = indices.map(|idx| block.rws[idx].stack_value());
        self.add_words.assign(region, offset, [a, b], c)?;
        self.is_sub.assign(
            region,
            offset,
            F::from(opcode.as_u64()),
            F::from(OpcodeId::SUB.as_u64()),
            F::from(OpcodeId::ADD.as_u64()),
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::evm_circuit::{
        execution::bus_mapping_tmp::{
            Block, Bytecode, Call, ExecStep, Rw, Transaction,
        },
        step::ExecutionState,
        test::{rand_word, run_test_circuit_incomplete_fixed_table},
        util::RandomLinearCombination,
    };
    use bus_mapping::{
        eth_types::{ToBigEndian, ToLittleEndian, Word},
        evm::OpcodeId,
    };
    use halo2::arithmetic::BaseExt;
    use pairing::bn256::Fr as Fp;

    fn test_ok(opcode: OpcodeId, a: Word, b: Word, c: Word) {
        let randomness = Fp::rand();
        let bytecode = Bytecode::new(
            [
                vec![OpcodeId::PUSH32.as_u8()],
                b.to_be_bytes().to_vec(),
                vec![OpcodeId::PUSH32.as_u8()],
                a.to_be_bytes().to_vec(),
                vec![opcode.as_u8(), OpcodeId::STOP.as_u8()],
            ]
            .concat(),
        );
        let block = Block {
            randomness,
            txs: vec![Transaction {
                calls: vec![Call {
                    id: 1,
                    is_root: false,
                    is_create: false,
                    opcode_source:
                        RandomLinearCombination::random_linear_combine(
                            bytecode.hash.to_le_bytes(),
                            randomness,
                        ),
                }],
                steps: vec![
                    ExecStep {
                        rw_indices: vec![0, 1, 2],
                        execution_state: ExecutionState::ADD,
                        rw_counter: 1,
                        program_counter: 66,
                        stack_pointer: 1022,
                        gas_left: 3,
                        gas_cost: 3,
                        opcode: Some(opcode),
                        ..Default::default()
                    },
                    ExecStep {
                        execution_state: ExecutionState::STOP,
                        rw_counter: 4,
                        program_counter: 67,
                        stack_pointer: 1023,
                        gas_left: 0,
                        opcode: Some(OpcodeId::STOP),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            rws: vec![
                Rw::Stack {
                    rw_counter: 1,
                    is_write: false,
                    call_id: 1,
                    stack_pointer: 1022,
                    value: a,
                },
                Rw::Stack {
                    rw_counter: 2,
                    is_write: false,
                    call_id: 1,
                    stack_pointer: 1023,
                    value: b,
                },
                Rw::Stack {
                    rw_counter: 3,
                    is_write: true,
                    call_id: 1,
                    stack_pointer: 1023,
                    value: c,
                },
            ],
            bytecodes: vec![bytecode],
        };
        assert_eq!(run_test_circuit_incomplete_fixed_table(block), Ok(()));
    }

    #[test]
    fn add_gadget_simple() {
        test_ok(
            OpcodeId::ADD,
            0x030201.into(),
            0x060504.into(),
            0x090705.into(),
        );
        test_ok(
            OpcodeId::SUB,
            0x090705.into(),
            0x060504.into(),
            0x030201.into(),
        );
    }

    #[test]
    fn add_gadget_rand() {
        let a = rand_word();
        let b = rand_word();
        test_ok(OpcodeId::ADD, a, b, a.overflowing_add(b).0);
        test_ok(OpcodeId::SUB, a, b, a.overflowing_sub(b).0);
    }
}
