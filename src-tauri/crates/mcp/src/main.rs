//! `otomo-mcp` — lets Claude read and listen to the Otomo song library.
//!
//! Claude Desktop starts this as a subprocess and speaks MCP over stdin and
//! stdout, so **stdout carries the protocol**: anything this program wants to
//! say to a human goes to stderr.

mod control;
mod edit_tools;
mod fetch;
mod library;
mod pending;
mod picture;
mod rpc;
mod tools;

fn main() {
    let stdin = std::io::stdin().lock();
    let stdout = std::io::stdout().lock();
    if let Err(e) = rpc::serve(&mut tools::Server, stdin, stdout) {
        eprintln!("otomo-mcp: {e}");
        std::process::exit(1);
    }
}
