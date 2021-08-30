// Doc this

use crate::evm::{
    EvmWord, GlobalCounter, Instruction, MemoryAddress, ProgramCounter,
    StackAddress, MEM_ADDR_ZERO,
};
use crate::{
    error::Error, evm::opcodes::Opcode,
    operation::container::OperationContainer,
};
use alloc::collections::BTreeMap;
use core::{convert::TryFrom, str::FromStr};
use halo2::arithmetic::FieldExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::OperationRef;

/// Represents a single step of an [`ExecutionTrace`](super::ExecutionTrace). It
/// contains all of the information relative to this step:
/// - Memory view at current execution step.`
/// - Stack view at current execution step.
/// - EVM [`Instruction`] executed in this step.
/// - [`ProgramCounter`] relative to this step.
/// - [`GlobalCounter`] assigned to this step by the program.
/// - Bus Mapping instances containing references to all of the
///   [`Operation`](crate::operation::Operation)s generated by this step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionStep {
    memory: BTreeMap<MemoryAddress, EvmWord>,
    stack: Vec<EvmWord>,
    instruction: Instruction,
    pc: ProgramCounter,
    gc: GlobalCounter,
    // Holds refs to the container with the related mem ops.
    bus_mapping_instance: Vec<OperationRef>,
}

impl ExecutionStep {
    /// Generate a new `ExecutionStep` from it's fields but with an empty
    /// bus-mapping instance vec.
    pub fn new(
        memory: BTreeMap<MemoryAddress, EvmWord>,
        stack: Vec<EvmWord>,
        instruction: Instruction,
        pc: ProgramCounter,
        gc: GlobalCounter,
    ) -> Self {
        ExecutionStep {
            memory,
            stack,
            instruction,
            pc,
            gc,
            bus_mapping_instance: Vec::new(),
        }
    }

    /// Returns the Memory view of this `ExecutionStep` in the form of a
    /// `BTreeMap`.
    pub const fn memory(&self) -> &BTreeMap<MemoryAddress, EvmWord> {
        &self.memory
    }

    /// Returns the Stack view of this `ExecutionStep` in the form of a Vector.
    pub const fn stack(&self) -> &Vec<EvmWord> {
        &self.stack
    }

    /// Returns the stack pointer at this execution step height.
    pub fn stack_addr(&self) -> StackAddress {
        // Stack has 1024 slots.
        // First allocation slot for us in the stack is 1023.
        StackAddress::from(1024 - self.stack.len())
    }

    /// Returns the last memory region written at this execution step height.
    pub fn memory_addr(&self) -> &MemoryAddress {
        self.memory
            .iter()
            .last()
            .map(|items| items.0)
            .unwrap_or_else(|| &MEM_ADDR_ZERO)
    }

    /// Returns the [`Instruction`] executed at this step.
    pub const fn instruction(&self) -> &Instruction {
        &self.instruction
    }

    /// Returns the [`ProgramCounter`] that corresponds to this step.
    pub const fn pc(&self) -> ProgramCounter {
        self.pc
    }

    /// Returns the [`GlobalCounter`] associated to this step's `Instuction`
    /// execution.
    pub const fn gc(&self) -> GlobalCounter {
        self.gc
    }

    /// Sets the global counter of the [`Instruction`] execution to the one sent
    /// in the params.
    pub(crate) fn set_gc(&mut self, gc: impl Into<GlobalCounter>) {
        self.gc = gc.into()
    }

    /// Returns a reference to the bus-mapping instance.
    pub const fn bus_mapping_instance(&self) -> &Vec<OperationRef> {
        &self.bus_mapping_instance
    }

    /// Returns a mutable reference to the bus-mapping instance.
    pub(crate) fn bus_mapping_instance_mut(
        &mut self,
    ) -> &mut Vec<OperationRef> {
        &mut self.bus_mapping_instance
    }

    /// Given a mutable reference to an [`OperationContainer`], generate all of
    /// it's [`Instruction`]-related Memory, Stack and Storage ops, and register
    /// them in the container. This function will not only add the ops to
    /// the [`OperationContainer`] but also get it's [`OperationRef`]s and add
    /// them to the bus-mapping instance of the step.
    ///
    /// ## Returns the #operations added by the
    /// [`OpcodeId`](crate::evm::OpcodeId) into the container.
    pub(crate) fn gen_associated_ops<F: FieldExt>(
        &mut self,
        container: &mut OperationContainer,
    ) -> usize {
        self.instruction()
            .opcode_id()
            .gen_associated_ops(self, container)
    }
}

impl<'a> TryFrom<(&ParsedExecutionStep<'a>, GlobalCounter)> for ExecutionStep {
    type Error = Error;

    fn try_from(
        parse_info: (&ParsedExecutionStep<'a>, GlobalCounter),
    ) -> Result<Self, Self::Error> {
        // Memory part
        let mut mem_map = BTreeMap::new();
        parse_info
            .0
            .memory
            .iter()
            .try_for_each(|(mem_addr, word)| {
                mem_map.insert(
                    MemoryAddress::from_str(mem_addr)?,
                    EvmWord::from_str(word)?,
                );
                Ok(())
            })?;

        // Stack part
        let mut stack = vec![];
        parse_info.0.stack.iter().try_for_each(|word| {
            stack.push(EvmWord::from_str(word)?);
            Ok(())
        })?;

        Ok(ExecutionStep::new(
            mem_map,
            stack,
            Instruction::from_str(parse_info.0.opcode)?,
            parse_info.0.pc,
            parse_info.1,
        ))
    }
}

/// Helper structure whose only purpose is to serve as a De/Serialization
/// derivation guide for the serde Derive macro.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[doc(hidden)]
pub struct ParsedExecutionStep<'a> {
    memory: HashMap<&'a str, &'a str>,
    stack: Vec<&'a str>,
    opcode: &'a str,
    pc: ProgramCounter,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evm::opcodes::ids::OpcodeId;
    use num::BigUint;

    #[test]
    fn parse_single_step() {
        let step_json = r#"
        {
            "memory": {
                "0": "0000000000000000000000000000000000000000000000000000000000000000",
                "20": "0000000000000000000000000000000000000000000000000000000000000000",
                "40": "0000000000000000000000000000000000000000000000000000000000000080"
            },
            "stack": [],
            "opcode": "JUMPDEST",
            "pc": 53
        }
        "#;

        let step_loaded: ExecutionStep = ExecutionStep::try_from((
            &serde_json::from_str::<ParsedExecutionStep>(step_json)
                .expect("Error on parsing"),
            GlobalCounter(0usize),
        ))
        .expect("Error on conversion");

        let expected_step = {
            let mut mem_map = BTreeMap::new();
            mem_map.insert(
                MemoryAddress(BigUint::from(0x00u8)),
                EvmWord(BigUint::from(0u8)),
            );
            mem_map.insert(
                MemoryAddress(BigUint::from(0x20u8)),
                EvmWord(BigUint::from(0u8)),
            );
            mem_map.insert(
                MemoryAddress(BigUint::from(0x40u8)),
                EvmWord(BigUint::from(0x80u8)),
            );

            ExecutionStep::new(
                mem_map,
                vec![],
                Instruction::new(OpcodeId::JUMPDEST, None),
                ProgramCounter(53),
                GlobalCounter(0),
            )
        };

        assert_eq!(step_loaded, expected_step)
    }
}
