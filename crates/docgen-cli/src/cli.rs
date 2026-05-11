use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "docgen", about = "Generate docstrings using a local vLLM instance.")]
pub struct Cli {
    /// File or folder to process
    pub target: PathBuf,

    /// Docstring format: mkdocs, tsdoc (auto-detected if omitted)
    #[arg(long = "format")]
    pub fmt: Option<String>,

    /// Recurse into subdirectories
    #[arg(long, short = 'r')]
    pub recursive: bool,

    /// Regenerate existing docstrings
    #[arg(long)]
    pub force: bool,
}
