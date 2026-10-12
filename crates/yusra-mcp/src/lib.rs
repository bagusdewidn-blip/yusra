//! Server MCP: tool baca di atas index proyek, transport stdio.

mod project;
mod schema;
mod server;
mod settings;

pub use project::{IndexStats, Project, STATE_DIR};
pub use schema::*;
pub use server::{YusraServer, serve_stdio};
pub use settings::{PathPolicy, Settings};
