use crate::evm_circuit::{
    util::constraint_builder::ConstraintBuilder,
    util::{math_gadget::*, CachedRegion},
};
use eth_types::Field;
use halo2_proofs::plonk::{Error, Expression};

/// Returns `1` when `lhs == rhs`, and returns `0` otherwise.
#[derive(Clone, Debug)]
pub struct IsEqualGadget<F> {
    is_zero: IsZeroGadget<F>,
}

impl<F: Field> IsEqualGadget<F> {
    pub(crate) fn construct(
        cb: &mut ConstraintBuilder<F>,
        lhs: Expression<F>,
        rhs: Expression<F>,
    ) -> Self {
        let is_zero = IsZeroGadget::construct(cb, lhs - rhs);

        Self { is_zero }
    }

    pub(crate) fn expr(&self) -> Expression<F> {
        self.is_zero.expr()
    }

    pub(crate) fn assign(
        &self,
        region: &mut CachedRegion<'_, '_, F>,
        offset: usize,
        lhs: F,
        rhs: F,
    ) -> Result<F, Error> {
        self.is_zero.assign(region, offset, lhs - rhs)
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::*;
    use crate::evm_circuit::util::{CachedRegion, Cell};
    use eth_types::*;
    use halo2_proofs::halo2curves::bn256::Fr;
    use halo2_proofs::plonk::Error;

    #[derive(Clone)]
    /// IsEqualGadgetTestContainer: require(a == b)
    struct IsEqualGadgetTestContainer<F> {
        eq_gadget: IsEqualGadget<F>,
        a: Cell<F>,
        b: Cell<F>,
    }

    impl<F: Field> MathGadgetContainer<F> for IsEqualGadgetTestContainer<F> {
        fn configure_gadget_container(cb: &mut ConstraintBuilder<F>) -> Self {
            let a = cb.query_cell();
            let b = cb.query_cell();
            let eq_gadget = IsEqualGadget::<F>::construct(cb, a.expr(), b.expr());
            cb.require_equal("Inputs must equal (a==b)", eq_gadget.expr(), 1.expr());
            IsEqualGadgetTestContainer { eq_gadget, a, b }
        }

        fn assign_gadget_container(
            &self,
            witnesses: &[Word],
            region: &mut CachedRegion<'_, '_, F>,
        ) -> Result<(), Error> {
            let a = witnesses[0].to_scalar().unwrap();
            let b = witnesses[1].to_scalar().unwrap();
            let offset = 0;

            self.a.assign(region, offset, Value::known(a))?;
            self.b.assign(region, offset, Value::known(b))?;
            self.eq_gadget.assign(region, offset, a, b)?;

            Ok(())
        }
    }

    #[test]
    fn test_isequal_0() {
        try_test!(
            IsEqualGadgetTestContainer<Fr>,
            vec![Word::from(0), Word::from(0)],
            true,
        );
    }

    #[test]
    fn test_isequal_1() {
        try_test!(
            IsEqualGadgetTestContainer<Fr>,
            vec![Word::from(1), Word::from(1)],
            true,
        );
    }

    #[test]
    fn test_isequal_1000() {
        try_test!(
            IsEqualGadgetTestContainer<Fr>,
            vec![Word::from(1000), Word::from(1000)],
            true,
        );
    }

    #[test]
    fn test_isequal_1_0() {
        try_test!(
            IsEqualGadgetTestContainer<Fr>,
            vec![Word::from(1), Word::from(0)],
            false,
        );
    }

    #[test]
    fn test_isequal_0_1() {
        try_test!(
            IsEqualGadgetTestContainer<Fr>,
            vec![Word::from(0), Word::from(1)],
            false,
        );
    }
}
