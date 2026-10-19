//! `pwrroute mcp` as a standalone binary, for development and the
//! end-to-end tests. The shipped command is the app binary's `mcp`
//! subcommand, which calls the same [`pwrroute_mcp::proxy::main`].

fn main() -> std::process::ExitCode {
    pwrroute_mcp::proxy::main()
}
