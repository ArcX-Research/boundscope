use boundscope::{config::Catalog, engine::Engine, mcp};
use clap::Parser;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Parser)]
#[command(version, about = "Boundscope MCP server over standard input/output")]
struct Args {
    #[arg(long)]
    config: PathBuf,
    #[arg(long)]
    store: PathBuf,
}

fn main() {
    let args = Args::parse();
    let shutdown = Arc::new(AtomicBool::new(false));
    let signal = shutdown.clone();
    let result = (|| {
        ctrlc::set_handler(move || signal.store(true, Ordering::Release))
            .map_err(|e| e.to_string())?;
        Catalog::load(&args.config)?;
        mcp::serve(Engine::new(args.config, args.store)?, shutdown)
    })();
    if let Err(error) = result {
        eprintln!("boundscope-mcp: {error}");
        std::process::exit(2);
    }
}
