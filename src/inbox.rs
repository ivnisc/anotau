use crate::people::UserId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TransferId(pub(crate) i64);

#[derive(Clone)]
pub(crate) struct Incoming {
    pub(crate) id: TransferId,
    pub(crate) sender: UserId,
    pub(crate) title: String,
}
