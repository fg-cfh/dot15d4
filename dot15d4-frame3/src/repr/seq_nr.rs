#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SeqNrRepr {
    Yes,
    No,
} // 1 byte

impl SeqNrRepr {
    pub const fn length(&self) -> u16 {
        match self {
            SeqNrRepr::Yes => 1,
            SeqNrRepr::No => 0,
        }
    }
}
