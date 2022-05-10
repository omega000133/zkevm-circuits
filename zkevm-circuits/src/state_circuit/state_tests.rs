mod tests {
    use super::super::StateCircuit;
    use crate::evm_circuit::table::AccountFieldTag;
    use crate::evm_circuit::witness::{Rw, RwMap};
    use bus_mapping::operation::{
        MemoryOp, Operation, OperationContainer, RWCounter, StackOp, StorageOp, RW,
    };
    use eth_types::{
        evm_types::{MemoryAddress, StackAddress},
        Address, ToAddress, Word, U256,
    };
    use halo2_proofs::{
        arithmetic::BaseExt,
        dev::{MockProver, VerifyFailure},
        pairing::bn256::Fr,
        plonk::{Circuit, ConstraintSystem},
    };

    fn test_state_circuit_ok(
        memory_ops: Vec<Operation<MemoryOp>>,
        stack_ops: Vec<Operation<StackOp>>,
        storage_ops: Vec<Operation<StorageOp>>,
    ) {
        let rw_map = RwMap::from(&OperationContainer {
            memory: memory_ops,
            stack: stack_ops,
            storage: storage_ops,
            ..Default::default()
        });

        let randomness = Fr::rand();
        let circuit = StateCircuit::new(randomness, rw_map);
        let power_of_randomness = circuit.instance();

        let prover = MockProver::<Fr>::run(19, &circuit, power_of_randomness).unwrap();
        let verify_result = prover.verify();
        assert_eq!(verify_result, Ok(()));
    }

    #[test]
    fn degree() {
        let mut meta = ConstraintSystem::<Fr>::default();
        StateCircuit::configure(&mut meta);
        assert_eq!(meta.degree(), 19);
    }

    #[test]
    fn state_circuit_simple_2() {
        let memory_op_0 = Operation::new(
            RWCounter::from(12),
            RW::WRITE,
            MemoryOp::new(1, MemoryAddress::from(0), 32),
        );
        let memory_op_1 = Operation::new(
            RWCounter::from(24),
            RW::READ,
            MemoryOp::new(1, MemoryAddress::from(0), 32),
        );

        let memory_op_2 = Operation::new(
            RWCounter::from(17),
            RW::WRITE,
            MemoryOp::new(1, MemoryAddress::from(1), 32),
        );
        let memory_op_3 = Operation::new(
            RWCounter::from(87),
            RW::READ,
            MemoryOp::new(1, MemoryAddress::from(1), 32),
        );

        let stack_op_0 = Operation::new(
            RWCounter::from(17),
            RW::WRITE,
            StackOp::new(1, StackAddress::from(1), Word::from(32)),
        );
        let stack_op_1 = Operation::new(
            RWCounter::from(87),
            RW::READ,
            StackOp::new(1, StackAddress::from(1), Word::from(32)),
        );

        let storage_op_0 = Operation::new(
            RWCounter::from(0),
            RW::WRITE,
            StorageOp::new(
                U256::from(100).to_address(),
                Word::from(0x40),
                Word::from(32),
                Word::zero(),
                1usize,
                Word::zero(),
            ),
        );
        let storage_op_1 = Operation::new(
            RWCounter::from(18),
            RW::WRITE,
            StorageOp::new(
                U256::from(100).to_address(),
                Word::from(0x40),
                Word::from(32),
                Word::from(32),
                1usize,
                Word::from(32),
            ),
        );
        let storage_op_2 = Operation::new(
            RWCounter::from(19),
            RW::WRITE,
            StorageOp::new(
                U256::from(100).to_address(),
                Word::from(0x40),
                Word::from(32),
                Word::from(32),
                1usize,
                Word::from(32),
            ),
        );

        test_state_circuit_ok(
            vec![memory_op_0, memory_op_1, memory_op_2, memory_op_3],
            vec![stack_op_0, stack_op_1],
            vec![storage_op_0, storage_op_1, storage_op_2],
        );
    }

    #[test]
    fn state_circuit_simple_6() {
        let memory_op_0 = Operation::new(
            RWCounter::from(12),
            RW::WRITE,
            MemoryOp::new(1, MemoryAddress::from(0), 32),
        );
        let memory_op_1 = Operation::new(
            RWCounter::from(13),
            RW::READ,
            MemoryOp::new(1, MemoryAddress::from(0), 32),
        );
        let storage_op_2 = Operation::new(
            RWCounter::from(19),
            RW::WRITE,
            StorageOp::new(
                U256::from(100).to_address(),
                Word::from(0x40),
                Word::from(32),
                Word::from(32),
                1usize,
                Word::from(32),
            ),
        );
        test_state_circuit_ok(vec![memory_op_0, memory_op_1], vec![], vec![storage_op_2]);
    }

    #[test]
    fn lexicographic_ordering_test_1() {
        let memory_op = Operation::new(
            RWCounter::from(12),
            RW::WRITE,
            MemoryOp::new(1, MemoryAddress::from(0), 32),
        );
        let storage_op = Operation::new(
            RWCounter::from(19),
            RW::WRITE,
            StorageOp::new(
                U256::from(100).to_address(),
                Word::from(0x40),
                Word::from(32),
                Word::from(32),
                1usize,
                Word::from(32),
            ),
        );
        test_state_circuit_ok(vec![memory_op], vec![], vec![storage_op]);
    }

    #[test]
    fn lexicographic_ordering_test_2() {
        let memory_op_0 = Operation::new(
            RWCounter::from(12),
            RW::WRITE,
            MemoryOp::new(1, MemoryAddress::from(0), 32),
        );
        let memory_op_1 = Operation::new(
            RWCounter::from(13),
            RW::WRITE,
            MemoryOp::new(1, MemoryAddress::from(0), 32),
        );
        test_state_circuit_ok(vec![memory_op_0, memory_op_1], vec![], vec![]);
    }

    #[test]
    fn first_access_for_stack_is_write() {
        let rows = vec![
            Rw::Stack {
                rw_counter: 24,
                is_write: true,
                call_id: 1,
                stack_pointer: 1022,
                value: U256::from(394500u64),
            },
            Rw::Stack {
                rw_counter: 25,
                is_write: false,
                call_id: 1,
                stack_pointer: 1022,
                value: U256::from(394500u64),
            },
        ];

        assert_eq!(verify(rows), Ok(()));
    }

    #[test]
    fn diff_1_problem_repro() {
        let rows = vec![
            Rw::Account {
                rw_counter: 1,
                is_write: true,
                account_address: Address::default(),
                field_tag: AccountFieldTag::CodeHash,
                value: U256::zero(),
                value_prev: U256::zero(),
            },
            Rw::Account {
                rw_counter: 2,
                is_write: true,
                account_address: Address::default(),
                field_tag: AccountFieldTag::CodeHash,
                value: U256::zero(),
                value_prev: U256::zero(),
            },
        ];

        assert_eq!(verify(rows), Ok(()));
    }

    fn verify(rows: Vec<Rw>) -> Result<(), Vec<VerifyFailure>> {
        let n_rows = rows.len();

        let randomness = Fr::rand();
        let circuit = StateCircuit { randomness, rows };
        let power_of_randomness = circuit.instance();

        MockProver::<Fr>::run(17, &circuit, power_of_randomness)
            .unwrap()
            .verify_at_rows(0..n_rows, 0..n_rows)
    }
}
