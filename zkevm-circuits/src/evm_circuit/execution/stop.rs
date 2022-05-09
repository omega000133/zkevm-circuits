use crate::{
    evm_circuit::{
        execution::ExecutionGadget,
        step::ExecutionState,
        util::{constraint_builder::ConstraintBuilder, CachedRegion, Cell},
        witness::{Block, Call, ExecStep, Transaction},
    },
    util::Expr,
};
use eth_types::Field;
use halo2_proofs::plonk::Error;

#[derive(Clone, Debug)]
pub(crate) struct StopGadget<F> {
    opcode: Cell<F>,
}

impl<F: Field> ExecutionGadget<F> for StopGadget<F> {
    const NAME: &'static str = "STOP";

    const EXECUTION_STATE: ExecutionState = ExecutionState::STOP;

    fn configure(cb: &mut ConstraintBuilder<F>) -> Self {
        let opcode = cb.query_cell();
        cb.opcode_lookup(opcode.expr(), 1.expr());

        // Other constraints are ignored now for STOP to serve as a mocking
        // terminator

        Self { opcode }
    }

    fn assign_exec_step(
        &self,
        region: &mut CachedRegion<'_, '_, F>,
        offset: usize,
        _: &Block<F>,
        _: &Transaction,
        _: &Call,
        step: &ExecStep,
    ) -> Result<(), Error> {
        let opcode = step.opcode.unwrap();
        self.opcode
            .assign(region, offset, Some(F::from(opcode.as_u64())))?;

        Ok(())
    }
}
