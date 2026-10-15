use boundscope::{Result, engine::Engine, report::Store, trace};
use clap::{Parser, Subcommand};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Parser)]
#[command(
    version,
    about = "Find memory errors in registered C/C++ programs and preserve reproducible evidence"
)]
struct Args {
    #[arg(long, default_value = "targets.json", global = true)]
    config: PathBuf,
    #[arg(long, default_value = "local/reports", global = true)]
    store: PathBuf,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Review registered programs, inputs and execution limits.
    ListTargets,
    /// Compile and run a trusted registered target with ASan and UBSan.
    Check { target: String },
    /// Run the original definition against current source; preserve the baseline.
    Retest { report_id: String },
    /// Inspect a saved report and verify its source/input fingerprints.
    Report { report_id: String },
    /// List saved report summaries, newest first.
    ListReports {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Inspect execution phases, including attempts that failed before a report.
    Trace { trace_id: String },
    /// List local execution traces, newest activity first.
    ListTraces {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

fn execute(args: Args, cancel: &AtomicBool) -> Result<i32> {
    let (value, exit) = match args.command {
        Action::Trace { trace_id } => (
            serde_json::to_value(trace::load(&Store::open(&args.store)?, &trace_id)?)
                .map_err(|e| e.to_string())?,
            0,
        ),
        Action::ListTraces { limit } => (
            serde_json::json!({"traces":trace::list(&Store::open(&args.store)?, limit)?}),
            0,
        ),
        Action::Report { report_id } => (
            serde_json::to_value(Store::open(&args.store)?.load(&report_id)?)
                .map_err(|e| e.to_string())?,
            0,
        ),
        Action::ListReports { limit } => (
            serde_json::json!({"reports":Store::open(&args.store)?.list(limit)?}),
            0,
        ),
        action => {
            let engine = Engine::new(args.config, args.store)?;
            match action {
                Action::ListTargets => (engine.list_targets()?, 0),
                Action::Check { target } => {
                    eprintln!(
                        "Building and checking the registered target; Ctrl-C cancels the active process group."
                    );
                    let report = engine.check(&target, cancel)?;
                    let code = match report.outcome.as_str() {
                        "no_errors_observed" => 0,
                        "findings_observed" => 1,
                        _ => 2,
                    };
                    (report.summary(), code)
                }
                Action::Retest { report_id } => {
                    eprintln!("Retesting against current source; the baseline stays unchanged.");
                    let report = engine.retest(&report_id, cancel)?;
                    let code = match report.outcome.as_str() {
                        "no_errors_observed" => 0,
                        "findings_observed" => 1,
                        _ => 2,
                    };
                    (report.summary(), code)
                }
                _ => unreachable!(),
            }
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    Ok(exit)
}

fn main() {
    let args = Args::parse();
    let cancel = Arc::new(AtomicBool::new(false));
    let signal = cancel.clone();
    if let Err(e) = ctrlc::set_handler(move || signal.store(true, Ordering::Release)) {
        eprintln!("boundscope: cannot install cancellation handler: {e}");
        std::process::exit(2);
    }
    let exit = match execute(args, &cancel) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("boundscope: {error}");
            2
        }
    };
    std::process::exit(exit);
}
