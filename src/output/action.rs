use clap::ValueEnum;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum BulkAction {
    Create,
    #[default]
    Index,
    Update,
    Upsert,
}
