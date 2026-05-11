mod cli;
mod detect;
mod resolver;

use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() {
    let _args = Cli::parse();
    println!("docgen scaffold OK");
}
