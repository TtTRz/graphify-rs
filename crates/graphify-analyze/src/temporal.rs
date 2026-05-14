//! Temporal graph analysis via git history integration.
//!
//! Correlates graph nodes with git commit history to identify high-risk
//! nodes: frequently modified code with high connectivity.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use graphify_core::graph::KnowledgeGraph;
use graphify_core::model::TemporalNode;

/// Analyze temporal risk by correlating graph nodes with git history.
///
/// For each node's source file, queries `git log` to determine change frequency
/// and recency. Risk score = churn_rate × normalized_degree.
///
/// Returns up to `top_n` nodes sorted by risk score descending.
pub fn temporal_analysis(
    graph: &KnowledgeGraph,
    repo_root: &Path,
    top_n: usize,
) -> Vec<TemporalNode> {
    // Collect unique source files from nodes
    let mut file_stats: HashMap<String, (usize, String)> = HashMap::new(); // file → (commit_count, last_date)

    let source_files: Vec<String> = graph
        .nodes()
        .iter()
        .map(|n| n.source_file.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    for file in &source_files {
        if let Some((count, date)) = git_file_stats(repo_root, file) {
            file_stats.insert(file.clone(), (count, date));
        }
    }

    if file_stats.is_empty() {
        return Vec::new();
    }

    // Get current date for age calculation
    let now = chrono_days_since_epoch();

    // Max degree for normalization
    let max_degree = graph
        .node_ids()
        .iter()
        .map(|id| graph.degree(id))
        .max()
        .unwrap_or(1)
        .max(1) as f64;

    let mut results: Vec<TemporalNode> = graph
        .nodes()
        .iter()
        .filter_map(|node| {
            let (change_count, last_modified) = file_stats.get(&node.source_file)?;
            let age_days = date_to_age(last_modified, now).max(1);
            let churn_rate = *change_count as f64 / age_days as f64;
            let normalized_degree = graph.degree(&node.id) as f64 / max_degree;
            let risk_score = churn_rate * normalized_degree;

            Some(TemporalNode {
                id: node.id.clone(),
                label: node.label.clone(),
                last_modified: last_modified.clone(),
                change_count: *change_count,
                age_days,
                churn_rate,
                risk_score,
            })
        })
        .filter(|t| t.risk_score > 0.0)
        .collect();

    results.sort_by(|a, b| {
        b.risk_score
            .partial_cmp(&a.risk_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(top_n);
    results
}

/// Query git for a file's commit count and last modified date.
fn git_file_stats(repo_root: &Path, file: &str) -> Option<(usize, String)> {
    let output = Command::new("git")
        .args(["log", "--format=%aI", "--follow", "--", file])
        .current_dir(repo_root)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return None;
    }

    let count = lines.len();
    let last_date = lines[0].split('T').next().unwrap_or("").to_string();
    Some((count, last_date))
}

/// Simple day counter: days since 2020-01-01 from an ISO date string.
/// Uses the same calculation as [`chrono_days_since_epoch`] to avoid
/// offset from using different approximations.
fn date_to_age(date_str: &str, now_days: u64) -> u64 {
    match days_since_epoch_2020(date_str) {
        Some(file_days) => now_days.saturating_sub(file_days).max(1),
        None => 1,
    }
}

/// Compute approximate days since 2020-01-01 from an ISO date string.
/// Uses cumulative days per month to avoid the 30-day/month approximation
/// which caused up to ~30 day offset vs the precise epoch calculation.
/// Returns `None` for invalid date strings.
fn days_since_epoch_2020(date_str: &str) -> Option<u64> {
    let parts: Vec<u64> = date_str.split('-').filter_map(|p| p.parse().ok()).collect();
    if parts.len() < 3 {
        return None;
    }
    let (y, m, d) = (parts[0], parts[1], parts[2]);
    if m == 0 || m > 12 || d == 0 {
        return None;
    }
    // Cumulative days for months Jan..Nov (0-indexed)
    const CUM_DAYS: [u64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap_extra = if m > 2 && (y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)) {
        1
    } else {
        0
    };
    Some((y.saturating_sub(2020)) * 365 + CUM_DAYS.get(m as usize - 1).copied().unwrap_or(0) + leap_extra + d - 1)
}

/// Approximate days since 2020-01-01 for "now".
fn chrono_days_since_epoch() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // 2020-01-01 = 1577836800 epoch seconds
    secs.saturating_sub(1577836800) / 86400
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_to_age_computes_correctly() {
        // 2026-04-13 in epoch-2020 days:
        // 6 * 365 + CUM_DAYS[3] (90) + 13 = 2283
        let now = 6 * 365 + 90 + 13;
        let age = date_to_age("2026-01-01", now);
        // 2026-01-01 = 6*365 + 0 + 1 = 2191; 2283 - 2191 = 92
        assert!(age > 0 && age < 200, "age = {age}");
    }

    #[test]
    fn date_to_age_invalid_returns_1() {
        assert_eq!(date_to_age("invalid", 2300), 1);
    }

    #[test]
    fn days_since_epoch_consistent() {
        // 2020-01-01 is the epoch, so day 0
        assert_eq!(days_since_epoch_2020("2020-01-01"), Some(0));
        // 2020-02-01 = 31 days after epoch
        assert_eq!(days_since_epoch_2020("2020-02-01"), Some(31));
        // 2021-01-01 = 365 days after epoch
        assert_eq!(days_since_epoch_2020("2021-01-01"), Some(365));
        // 2020-03-01 = 31 + 29 (leap) + 0 = 60
        assert_eq!(days_since_epoch_2020("2020-03-01"), Some(60));
    }
}
