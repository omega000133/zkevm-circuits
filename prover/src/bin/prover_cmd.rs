use bus_mapping::circuit_input_builder::BuilderClient;
use bus_mapping::rpc::GethClient;
use env_logger::Env;
use ethers_providers::Http;
use halo2::{
    plonk::*,
    poly::commitment::Params,
    transcript::{Blake2bWrite, Challenge255},
};
use pairing::bn256::{Fr, G1Affine};
use std::env::var;
use std::fs::File;
use std::io::BufReader;
use std::str::FromStr;
use zkevm_circuits::evm_circuit::{
    table::FixedTableTag, test::TestCircuit, witness::block_convert,
};
use zkevm_circuits::state_circuit::StateCircuit;

#[derive(serde::Serialize)]
pub struct Proofs {
    state_proof: eth_types::Bytes,
    evm_proof: eth_types::Bytes,
}

/// This command generates and prints the proofs to stdout.
/// Required environment variables:
/// - BLOCK_NUM - the block number to generate the proof for
/// - RPC_URL - a geth http rpc that supports the debug namespace
/// - PARAMS_PATH - a path to a file generated with the gen_params tool
// TODO: move the proof generation into a module once we implement a rpc daemon for generating
// proofs.
#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let block_num: u64 = var("BLOCK_NUM")
        .expect("BLOCK_NUM env var")
        .parse()
        .expect("Cannot parse BLOCK_NUM env var");
    let rpc_url: String = var("RPC_URL")
        .expect("RPC_URL env var")
        .parse()
        .expect("Cannot parse RPC_URL env var");
    let params_path: String = var("PARAMS_PATH")
        .expect("PARAMS_PATH env var")
        .parse()
        .expect("Cannot parse PARAMS_PATH env var");

    // load polynomial commitment parameters
    let params_fs = File::open(&params_path).expect("couldn't open params");
    let params: Params<G1Affine> =
        Params::read::<_>(&mut BufReader::new(params_fs)).expect("Failed to read params");

    // request & build the inputs for the circuits
    let geth_client = GethClient::new(Http::from_str(&rpc_url).expect("GethClient from RPC_URL"));
    let builder = BuilderClient::new(geth_client)
        .await
        .expect("BuilderClient from GethClient");
    let builder = builder
        .gen_inputs(block_num)
        .await
        .expect("gen_inputs for BLOCK_NUM");

    // TODO: only {evm,state}_proof are implemented right now
    let evm_proof;
    let state_proof;
    let block = block_convert(&builder.block, &builder.code_db);
    {
        // generate evm_circuit proof
        let circuit = TestCircuit::<Fr>::new(block.clone(), FixedTableTag::iterator().collect());

        // TODO: can this be pre-generated to a file?
        // related
        // https://github.com/zcash/halo2/issues/443
        // https://github.com/zcash/halo2/issues/449
        let vk = keygen_vk(&params, &circuit).expect("keygen_vk for params, evm_circuit");
        let pk = keygen_pk(&params, vk, &circuit).expect("keygen_pk for params, vk, evm_circuit");

        // create a proof
        let mut transcript = Blake2bWrite::<_, _, Challenge255<_>>::init(vec![]);
        create_proof(&params, &pk, &[circuit], &[], &mut transcript).expect("evm proof");
        evm_proof = transcript.finalize();
    }

    {
        // generate state_circuit proof
        //
        // TODO: this should be configurable
        const MEMORY_ADDRESS_MAX: usize = 2000;
        const STACK_ADDRESS_MAX: usize = 1300;
        const MEMORY_ROWS_MAX: usize = 16384;
        const STACK_ROWS_MAX: usize = 16384;
        const STORAGE_ROWS_MAX: usize = 16384;
        const GLOBAL_COUNTER_MAX: usize = MEMORY_ROWS_MAX + STACK_ROWS_MAX + STORAGE_ROWS_MAX;

        let circuit = StateCircuit::<
            Fr,
            true,
            GLOBAL_COUNTER_MAX,
            MEMORY_ROWS_MAX,
            MEMORY_ADDRESS_MAX,
            STACK_ROWS_MAX,
            STACK_ADDRESS_MAX,
            STORAGE_ROWS_MAX,
        >::new_from_rw_map(block.randomness, &block.rws);

        // TODO: same quest like in the first scope
        let vk = keygen_vk(&params, &circuit).expect("keygen_vk for params, state_circuit");
        let pk = keygen_pk(&params, vk, &circuit).expect("keygen_pk for params, vk, state_circuit");

        // create a proof
        let mut transcript = Blake2bWrite::<_, _, Challenge255<_>>::init(vec![]);
        create_proof(&params, &pk, &[circuit], &[], &mut transcript).expect("state proof");
        state_proof = transcript.finalize();
    }

    serde_json::to_writer(
        std::io::stdout(),
        &Proofs {
            evm_proof: evm_proof.into(),
            state_proof: state_proof.into(),
        },
    )
    .expect("serialize and write");
}
