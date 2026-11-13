mod app;

use find_dev_data::{
    export, rules,
    scanner::{self, Event, ScanOptions},
};
use std::{
    env,
    fs::File,
    io::{self, BufWriter},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};

fn main() -> eframe::Result {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if !args.is_empty() {
        if let Err(error) = cli(&args) {
            eprintln!("find-dev-data: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([960.0, 640.0])
            .with_title("Find Dev Data"),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Find Dev Data",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}

fn cli(args: &[std::ffi::OsString]) -> io::Result<()> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!(
            "Find Dev Data\n\nLaunch the desktop app:\n  find-dev-data\n\nScan without a window (CSV to stdout by default):\n  find-dev-data --scan /path [--csv /path/report.csv] [--include-volumes]\n\nmacOS protected folders require Full Disk Access for the app or terminal."
        );
        return Ok(());
    }
    let mut root = None;
    let mut csv = None;
    let mut include_volumes = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].to_str() {
            Some("--scan" | "--csv") if i + 1 < args.len() => {
                if args[i] == "--scan" {
                    root = Some(PathBuf::from(&args[i + 1]));
                } else {
                    csv = Some(PathBuf::from(&args[i + 1]));
                }
                i += 2;
            }
            Some("--include-volumes") => {
                include_volumes = true;
                i += 1;
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Usage: find-dev-data --scan PATH [--csv FILE] [--include-volumes]",
                ));
            }
        }
    }
    let home = env::var_os("HOME").map(PathBuf::from);
    let root =
        root.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing --scan PATH"))?;
    let root = home
        .as_ref()
        .map_or(root.clone(), |home| rules::expand_home(&root, home));
    let mut findings = Vec::new();
    scanner::scan(
        ScanOptions {
            root,
            home,
            include_volumes,
        },
        Arc::new(AtomicBool::new(false)),
        |event| match event {
            Event::Found(finding) => scanner::merge_finding(&mut findings, finding),
            Event::Finished { progress, .. } => eprintln!(
                "Found {} folders in {:.1}s; {} inaccessible entries",
                findings.len(),
                progress.elapsed.as_secs_f64(),
                progress.errors
            ),
            Event::Warning(error) => eprintln!("{}: {}", error.path.display(), error.message),
            _ => {}
        },
    )?;
    findings.sort_by_key(|item| std::cmp::Reverse(item.logical_bytes));
    let refs: Vec<_> = findings.iter().collect();
    if let Some(path) = csv {
        export::write_csv(BufWriter::new(File::create(path)?), &refs)
    } else {
        export::write_csv(io::stdout().lock(), &refs)
    }
}
