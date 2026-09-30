//! palette: reads, checks and writes the palette documents of one project.
//!
//! The library holds the whole implementation; `main.rs` only parses the command
//! line and starts the MCP server. See `spec/palette-server.rst` in
//! `docs/changeset/rfc-0004.rst` for the contract.

pub mod backlog;
pub mod changeset;
pub mod cli;
pub mod directives;
pub mod docs;
pub mod edit;
pub mod errors;
pub mod generated;
pub mod layout;
pub mod lint;
pub mod ops;
pub mod params;
pub mod project;
pub mod records;
pub mod rst;
pub mod schema;
pub mod server;
pub mod state;
pub mod status;
pub mod text;
pub mod tools;
pub mod util;
pub mod vfs;
