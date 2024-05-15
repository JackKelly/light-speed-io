use crate::opcode::OpCode;

/// The u64 io_uring user_data represents the index_of_op in the left-most 32 bits,
/// and represents the io_uring opcode CODE in the right-most 32 bits.
#[derive(Debug)]
pub(crate) struct UringUserData {
    index_of_op: u32,
    op: OpCode,
}

impl UringUserData {
    pub(crate) fn new(index_of_op: usize, op: u8) -> Self {
        Self {
            index_of_op: index_of_op.try_into().unwrap(),
            op: OpCode::new(op),
        }
    }

    pub(crate) const fn index_of_op(&self) -> u32 {
        self.index_of_op
    }

    pub(crate) const fn opcode(&self) -> &OpCode {
        &self.op
    }
}

impl From<u64> for UringUserData {
    fn from(value: u64) -> Self {
        let index_of_op: u32 = (value >> 32).try_into().unwrap();
        let op = OpCode::new((value & 0xFF).try_into().unwrap());
        Self { index_of_op, op }
    }
}

impl Into<u64> for UringUserData {
    fn into(self) -> u64 {
        let index_of_op: u64 = (self.index_of_op as u64) << 32;
        index_of_op | self.op.value() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uring_user_data_round_trip() {
        const INDEX: usize = 100;
        const OPCODE: u8 = io_uring::opcode::Read::CODE;
        let uring_user_data = UringUserData::new(INDEX, OPCODE);
        let user_data_u64: u64 = uring_user_data.into();
        let uring_user_data = UringUserData::from(user_data_u64);
        assert_eq!(uring_user_data.index_of_op, INDEX as u32);
        assert_eq!(uring_user_data.op, OpCode::new(OPCODE));
    }
}
