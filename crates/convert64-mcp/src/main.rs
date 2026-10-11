//! MCP server over stdio exposing the `convert64-core` command registry as tools (`docs/TAD.md §4`).
//!
//! Reads newline-delimited JSON-RPC 2.0 messages from stdin and writes one response line
//! per request to stdout; see [`convert64_mcp::Server`].

use std::io::{BufRead, Write};

fn main() {
    let mut server = convert64_mcp::Server::default();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle_line(&line)
            && writeln!(stdout, "{reply}")
                .and_then(|()| stdout.flush())
                .is_err()
        {
            break;
        }
    }
}
