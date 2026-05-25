use clap::ValueEnum;

/// Sort-order values accepted by `--sort-order`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SortOrderArg {
    #[value(name = "asc")]
    Asc,
    #[value(name = "desc")]
    Desc,
}
