//use ignore::{types::TypesBuilder, WalkBuilder};
//use grep::{regex::RegexMatcher, searcher::Searcher, searcher::sinks};
//use taskchampion::{storage::AccessMode, Operations, Replica, Status, StorageConfig, Uuid};
//use regex::Regex;
use colored::Colorize;
use anyhow::{Result, anyhow};

mod config;
mod taskparser;

fn main() -> Result<()> {
    let cfg = config::get(); 

    println!("{:?}", cfg);

    Ok(())

}
