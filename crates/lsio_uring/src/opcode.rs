use std::fmt;

use io_uring::opcode;

/// Simple wrapper around io_uring opcode::*::CODE;
#[derive(PartialEq)]
pub(crate) struct OpCode(u8);

impl OpCode {
    pub(crate) const fn new(op: u8) -> Self {
        Self(op)
    }

    pub(crate) fn name(&self) -> &'static str {
        match self.0 {
            opcode::OpenAt::CODE => "openat",
            opcode::Read::CODE => "read",
            opcode::Close::CODE => "close",
            _ => "Un-recognised opcode",
        }
    }

    pub(crate) fn value(&self) -> u8 {
        self.0
    }
}

impl fmt::Debug for OpCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("OpCode")
            .field(&self.0)
            .field(&self.name())
            .finish()
    }
}
