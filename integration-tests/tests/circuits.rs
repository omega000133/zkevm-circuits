#![cfg(feature = "circuits")]

use bus_mapping::circuit_input_builder::{BuilderClient, CircuitsParams};
use eth_types::geth_types;
use halo2_proofs::{
    arithmetic::CurveAffine,
    dev::MockProver,
    halo2curves::{
        bn256::Fr,
        group::{Curve, Group},
    },
};
use integration_tests::{get_client, log_init, GenDataOutput, CHAIN_ID};
use lazy_static::lazy_static;
use log::trace;
use paste::paste;
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha20Rng;
use std::marker::PhantomData;
use zkevm_circuits::bytecode_circuit::dev::test_bytecode_circuit;
use zkevm_circuits::copy_circuit::dev::test_copy_circuit;
use zkevm_circuits::evm_circuit::witness::RwMap;
use zkevm_circuits::evm_circuit::{test::run_test_circuit, witness::block_convert};
use zkevm_circuits::state_circuit::StateCircuit;
use zkevm_circuits::super_circuit::SuperCircuit;
use zkevm_circuits::tx_circuit::{sign_verify::SignVerifyChip, Secp256k1Affine, TxCircuit};

lazy_static! {
    pub static ref GEN_DATA: GenDataOutput = GenDataOutput::load();
}

const CIRCUITS_PARAMS: CircuitsParams = CircuitsParams {
    max_rws: 16384,
    max_txs: 4,
    keccak_padding: None,
};

async fn test_evm_circuit_block(block_num: u64) {
    log::info!("test evm circuit, block number: {}", block_num);
    let cli = get_client();
    let cli = BuilderClient::new(cli, CIRCUITS_PARAMS).await.unwrap();
    let (builder, _) = cli.gen_inputs(block_num).await.unwrap();

    let block = block_convert(&builder.block, &builder.code_db);
    run_test_circuit(block).expect("evm_circuit verification failed");
}

async fn test_state_circuit_block(block_num: u64) {
    log::info!("test state circuit, block number: {}", block_num);
    let cli = get_client();
    let cli = BuilderClient::new(cli, CIRCUITS_PARAMS).await.unwrap();
    let (builder, _) = cli.gen_inputs(block_num).await.unwrap();

    // Generate state proof
    // log via trace some of the container ops for debugging purposes
    let stack_ops = builder.block.container.sorted_stack();
    trace!("stack_ops: {:#?}", stack_ops);
    let memory_ops = builder.block.container.sorted_memory();
    trace!("memory_ops: {:#?}", memory_ops);
    let storage_ops = builder.block.container.sorted_storage();
    trace!("storage_ops: {:#?}", storage_ops);

    const DEGREE: usize = 17;

    let rw_map = RwMap::from(&builder.block.container);

    let circuit = StateCircuit::<Fr>::new(rw_map, 1 << 16);
    let prover = MockProver::<Fr>::run(DEGREE as u32, &circuit, circuit.instance()).unwrap();
    prover
        .verify_par()
        .expect("state_circuit verification failed");
}

async fn test_tx_circuit_block(block_num: u64) {
    const DEGREE: u32 = 20;

    log::info!("test tx circuit, block number: {}", block_num);
    let cli = get_client();
    let cli = BuilderClient::new(cli, CIRCUITS_PARAMS).await.unwrap();

    let (_, eth_block) = cli.gen_inputs(block_num).await.unwrap();
    let txs: Vec<_> = eth_block
        .transactions
        .iter()
        .map(geth_types::Transaction::from)
        .collect();

    let mut rng = ChaCha20Rng::seed_from_u64(2);
    let aux_generator = <Secp256k1Affine as CurveAffine>::CurveExt::random(&mut rng).to_affine();

    let circuit = TxCircuit::<Fr, 4, { 4 * (4 + 32 + 32) }> {
        sign_verify: SignVerifyChip {
            aux_generator,
            window_size: 2,
            _marker: PhantomData,
        },
        txs,
        chain_id: CHAIN_ID,
    };

    let prover = MockProver::run(DEGREE, &circuit, vec![vec![]]).unwrap();

    prover.verify_par().expect("tx_circuit verification failed");
}

pub async fn test_bytecode_circuit_block(block_num: u64) {
    const DEGREE: u32 = 16;

    log::info!("test bytecode circuit, block number: {}", block_num);
    let cli = get_client();
    let cli = BuilderClient::new(cli, CIRCUITS_PARAMS).await.unwrap();
    let (builder, _) = cli.gen_inputs(block_num).await.unwrap();
    let bytecodes: Vec<Vec<u8>> = builder.code_db.0.values().cloned().collect();

    test_bytecode_circuit::<Fr>(DEGREE, bytecodes);
}

pub async fn test_copy_circuit_block(block_num: u64) {
    const DEGREE: u32 = 16;

    log::info!("test copy circuit, block number: {}", block_num);
    let cli = get_client();
    let cli = BuilderClient::new(cli, CIRCUITS_PARAMS).await.unwrap();
    let (builder, _) = cli.gen_inputs(block_num).await.unwrap();
    let block = block_convert(&builder.block, &builder.code_db);

    assert!(test_copy_circuit(DEGREE, block).is_ok());
}

pub async fn test_super_circuit_block(block_num: u64) {
    const MAX_TXS: usize = 4;
    const MAX_CALLDATA: usize = 512;
    const MAX_RWS: usize = 5888;

    log::info!("test super circuit, block number: {}", block_num);
    let cli = get_client();
    let cli = BuilderClient::new(
        cli,
        CircuitsParams {
            max_rws: MAX_RWS,
            max_txs: MAX_TXS,
            keccak_padding: None,
        },
    )
    .await
    .unwrap();
    let (builder, eth_block) = cli.gen_inputs(block_num).await.unwrap();
    let (k, circuit, instance) =
        SuperCircuit::<_, MAX_TXS, MAX_CALLDATA, MAX_RWS>::build_from_circuit_input_builder(
            &builder,
            eth_block,
            &mut ChaCha20Rng::seed_from_u64(2),
        )
        .unwrap();
    let prover = MockProver::run(k, &circuit, instance).unwrap();
    let res = prover.verify_par();
    if let Err(err) = res {
        eprintln!("Verification failures:");
        eprintln!("{:#?}", err);
        panic!("Failed verification");
    }
}

macro_rules! declare_tests {
    ($name:ident, $block_tag:expr) => {
        paste! {
            #[tokio::test]
            async fn [<serial_test_evm_ $name>]() {
                log_init();
                let block_num = GEN_DATA.blocks.get($block_tag).unwrap();
                test_evm_circuit_block(*block_num).await;
            }

            #[tokio::test]
            async fn [<serial_test_state_ $name>]() {
                log_init();
                let block_num = GEN_DATA.blocks.get($block_tag).unwrap();
                test_state_circuit_block(*block_num).await;
            }

            #[tokio::test]
            async fn [<serial_test_tx_ $name>]() {
                log_init();
                let block_num = GEN_DATA.blocks.get($block_tag).unwrap();
                test_tx_circuit_block(*block_num).await;
            }

            #[tokio::test]
            async fn [<serial_test_bytecode_ $name>]() {
                log_init();
                let block_num = GEN_DATA.blocks.get($block_tag).unwrap();
                test_bytecode_circuit_block(*block_num).await;
            }

            #[tokio::test]
            async fn [<serial_test_copy_ $name>]() {
                log_init();
                let block_num = GEN_DATA.blocks.get($block_tag).unwrap();
                test_copy_circuit_block(*block_num).await;
            }

            #[tokio::test]
            async fn [<serial_test_super_ $name>]() {
                log_init();
                let block_num = GEN_DATA.blocks.get($block_tag).unwrap();
                test_super_circuit_block(*block_num).await;
            }
        }
    };
}

declare_tests!(circuit_block_transfer_0, "Transfer 0");
/*
declare_tests!(
    circuit_deploy_greeter,
    "Deploy Greeter"
);
*/
declare_tests!(circuit_multiple_transfers_0, "Multiple transfers 0");
declare_tests!(
    circuit_erc20_openzeppelin_transfer_fail,
    "ERC20 OpenZeppelin transfer failed"
);
declare_tests!(
    circuit_erc20_openzeppelin_transfer_succeed,
    "ERC20 OpenZeppelin transfer successful"
);
declare_tests!(
    circuit_multiple_erc20_openzeppelin_transfers,
    "Multiple ERC20 OpenZeppelin transfers"
);
