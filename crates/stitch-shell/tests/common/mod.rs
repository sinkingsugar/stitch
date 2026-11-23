//! What shell tests share.

/// `output` without bash's `bash: child setpgid (N to N): Operation not
/// permitted` lines, for comparing it exactly. bash calls `setpgid` for a
/// new job in both the shell and the job, and on macOS the two racing now
/// and then fail the job's (the shell's call gives it its group): bash
/// says so in the output, and a session keeps it (see the shell helper's
/// docs). Only that line, whole, is dropped.
pub fn without_setpgid(output: &str) -> String {
  output
    .lines()
    .filter(|l| !setpgid_line(l))
    .collect::<Vec<_>>()
    .join("\n")
}

fn setpgid_line(line: &str) -> bool {
  let pids = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
  line
    .strip_prefix("bash: child setpgid (")
    .and_then(|l| l.strip_suffix("): Operation not permitted"))
    .and_then(|l| l.split_once(" to "))
    .is_some_and(|(a, b)| pids(a) && pids(b))
}
