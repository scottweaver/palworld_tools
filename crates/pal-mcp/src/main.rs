//! MCP frontend: a local stdio server exposing the breeding
//! calculator to MCP clients (Claude Desktop / Cowork, Claude Code).
//!
//! Usage: `pal-mcp <save-or-pals-file>` — the same pool sources the
//! TUI accepts (`Level.sav` or a pals TOML). Diagnostics go to
//! stderr, which MCP clients surface in their server logs; stdout
//! belongs to the protocol.

mod output;
mod server;

use anyhow::{Context, Result};
use pal_solver::child::ChildIndex;
use pal_solver::iv::IvOdds;
use pal_solver::passives::PassiveOdds;
use pal_solver::search::Solver;
use rmcp::ServiceExt;
use rmcp::transport::stdio;

use crate::server::PalMcpServer;

#[tokio::main]
async fn main() -> Result<()> {
    let pool_path = std::env::args()
        .nth(1)
        .context("usage: pal-mcp <save-or-pals-file>")?;

    // Leaked deliberately: the database and solver live for the whole
    // process and are shared with blocking search tasks, which need
    // 'static borrows.
    let db: &'static pal_core::model::PalDb = Box::leak(Box::new(
        pal_core::vendored::pal_db().context("parsing the embedded db.json")?,
    ));
    let breeding =
        pal_core::vendored::breeding_db(db).context("parsing the embedded breeding.json")?;
    let index: &'static ChildIndex = Box::leak(Box::new(
        ChildIndex::build(&breeding).context("building the child index")?,
    ));
    let odds: &'static PassiveOdds = Box::leak(Box::new(
        PassiveOdds::from_mechanics(db.mechanics()).context("deriving passive odds")?,
    ));
    let iv_odds: &'static IvOdds = Box::leak(Box::new(
        IvOdds::from_mechanics(db.mechanics()).context("deriving IV odds")?,
    ));
    let solver: &'static Solver<'static> =
        Box::leak(Box::new(Solver::new(db, index, odds, iv_odds)));

    let (owned, status) = match pal_pool::load(&pool_path, db)? {
        pal_pool::Loaded::Pool { owned, status } => (owned, status),
        pal_pool::Loaded::Missing => (
            Vec::new(),
            format!("{pool_path} not found — starting with an empty pool (reload_pool retries)"),
        ),
    };
    eprintln!("pal-mcp: {status}");

    let service = PalMcpServer::new(solver, pool_path, owned, status)
        .serve(stdio())
        .await
        .context("starting the MCP stdio server")?;
    service.waiting().await.context("serving MCP requests")?;
    Ok(())
}
