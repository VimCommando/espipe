use clap::ValueEnum;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum BulkAction {
    Create,
    Index,
    Update,
}

impl Default for BulkAction {
    fn default() -> Self {
        Self::Create
    }
}
