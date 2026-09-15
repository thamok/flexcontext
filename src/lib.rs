//! Structural lexical source-code retrieval.

pub mod cache;
pub mod index;
pub mod language;
pub mod lexical;
pub mod model;
pub mod output;
pub mod parser;
pub mod ranking;
pub mod relations;
pub mod repository;
pub mod search;
pub mod selection;

pub use model::{SearchOptions, SearchResponse};
pub use search::{SearchSession, search};
pub mod mcp;

pub mod slicing;
