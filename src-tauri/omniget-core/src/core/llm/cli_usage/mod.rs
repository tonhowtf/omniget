pub mod activity;
pub mod parse;
pub mod report;
pub mod scan;
pub mod windows;

pub use parse::{CliKind, CliUsageEntry, FileParser, Parsed, RateLimitSample};
pub use report::{CliUsageReport, ReportOptions, SessionSummary, ToolHeat, Totals};
pub use scan::{
    scan, scan_roots, walk_jsonl, ScanOutcome, ScanRoot, ScanState, ScanStats, ERR_CLI_USAGE_STATE,
};
pub use windows::{
    merge_windows, CapacityRecord, CapacityWindow, PlanLimits, UsageWindow, WindowKind,
    WindowSource,
};

use std::path::PathBuf;

pub fn state_path() -> Option<PathBuf> {
    super::roster_store::llm_dir().map(|d| d.join("cli-usage-state.json"))
}

pub fn capacity_path() -> Option<PathBuf> {
    super::roster_store::llm_dir().map(|d| d.join("cli-capacity.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cold_sweep_stays_inside_the_budget() {
        let mb: u64 = std::env::var("OMNIGET_CLI_USAGE_BENCH_MB")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8);
        let dir = std::env::temp_dir().join(format!(
            "omniget-cli-usage-bench-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = write_synthetic_history(&dir, mb * 1024 * 1024);

        let mut state = ScanState::new();
        let started = std::time::Instant::now();
        let out = scan_roots(&[ScanRoot::new(CliKind::Claude, "bench", &dir)], &mut state);
        let cold = started.elapsed();

        let warm_started = std::time::Instant::now();
        let warm = scan_roots(&[ScanRoot::new(CliKind::Claude, "bench", &dir)], &mut state);
        let warm_elapsed = warm_started.elapsed();
        std::fs::remove_dir_all(&dir).ok();

        let mb_per_s = bytes as f64 / 1024.0 / 1024.0 / cold.as_secs_f64();
        let projected_500mb_ms = 500.0 / mb_per_s * 1000.0;
        println!(
            "[cli_usage bench] {} MB in {:?} ({:.0} MB/s, {} entries) -> 500 MB projected {:.0} ms; warm pass {:?}",
            bytes / 1024 / 1024,
            cold,
            mb_per_s,
            out.entries.len(),
            projected_500mb_ms,
            warm_elapsed
        );
        assert!(
            warm.entries.is_empty(),
            "a warm sweep must read nothing new"
        );
        if std::env::var("OMNIGET_CLI_USAGE_BENCH_STRICT").is_ok() {
            assert!(
                warm_elapsed.as_millis() <= 20,
                "warm budget is 20 ms, got {warm_elapsed:?}"
            );
            assert!(
                projected_500mb_ms <= 200.0,
                "cold budget is 200 ms for 500 MB, projected {projected_500mb_ms:.0} ms"
            );
        }
    }

    fn write_synthetic_history(dir: &std::path::Path, target_bytes: u64) -> u64 {
        use std::io::Write;
        let filler = "lorem ipsum dolor sit amet ".repeat(80);
        let mut written = 0u64;
        let mut file_index = 0;
        while written < target_bytes {
            let path = dir.join(format!("bench-{file_index}.jsonl"));
            let file = std::fs::File::create(&path).unwrap();
            let mut w = std::io::BufWriter::with_capacity(1 << 20, file);
            let mut this_file = 0u64;
            let mut i = 0u64;
            while this_file < 25 * 1024 * 1024 && written + this_file < target_bytes {
                let line = if i.is_multiple_of(8) {
                    format!(
                        r#"{{"type":"assistant","isSidechain":false,"sessionId":"s{file_index}","timestamp":"2026-03-04T12:00:00.000Z","requestId":"req_{file_index}_{i}","message":{{"id":"msg_{file_index}_{i}","role":"assistant","model":"claude-opus-5","content":[{{"type":"text","text":"{filler}"}},{{"type":"tool_use","id":"t","name":"Bash","input":{{}}}}],"usage":{{"input_tokens":12,"cache_creation_input_tokens":2000,"cache_read_input_tokens":30000,"output_tokens":300,"output_tokens_details":{{"thinking_tokens":0}}}}}}}}"#
                    )
                } else {
                    format!(
                        r#"{{"type":"user","sessionId":"s{file_index}","timestamp":"2026-03-04T12:00:00.000Z","uuid":"u{i}","toolUseResult":{{"stdout":"{filler}","stderr":"","interrupted":false}}}}"#
                    )
                };
                writeln!(w, "{line}").unwrap();
                this_file += line.len() as u64 + 1;
                i += 1;
            }
            w.flush().unwrap();
            written += this_file;
            file_index += 1;
        }
        written
    }

    #[test]
    fn the_state_and_capacity_files_sit_next_to_the_roster() {
        if let Some(dir) = super::super::roster_store::llm_dir() {
            assert_eq!(state_path().unwrap(), dir.join("cli-usage-state.json"));
            assert_eq!(capacity_path().unwrap(), dir.join("cli-capacity.json"));
        }
    }
}
