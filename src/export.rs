use crate::scanner::Finding;
use std::io::{self, Write};

pub fn write_csv(mut output: impl Write, findings: &[&Finding]) -> io::Result<()> {
    writeln!(
        output,
        "tool,category,path,logical_bytes,allocated_bytes,files,complete,errors,symlinks_skipped"
    )?;
    for item in findings {
        writeln!(
            output,
            "{},{},{},{},{},{},{},{},{}",
            field(&item.tool),
            field(item.category.label()),
            field(&item.path.to_string_lossy()),
            item.logical_bytes,
            item.allocated_bytes,
            item.files,
            item.complete,
            item.errors,
            item.symlinks_skipped
        )?;
    }
    output.flush()
}

fn field(value: &str) -> String {
    // Prevent spreadsheet programs from interpreting a filename or path as a formula.
    let prefix = if value.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        "'"
    } else {
        ""
    };
    format!("\"{prefix}{}\"", value.replace('"', "\"\""))
}
