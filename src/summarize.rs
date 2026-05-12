//! Python subprocess wrappers for the bundled `tools/scripts/*.py` pipeline.
//!
//! All entry points spawn the user's configured Python venv (`settings.python_venv_path`)
//! and stream stdout line-by-line so callers can pipe progress to the tray UI.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;

pub use crate::config::find_python;

/// Read stderr to completion in a background thread so the child does not block on a full pipe
/// when stdout streaming finishes first. Returns a join handle producing the captured bytes.
fn drain_stderr(child: &mut Child) -> Option<thread::JoinHandle<Vec<u8>>> {
    child.stderr.take().map(|mut err| {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err.read_to_end(&mut buf);
            buf
        })
    })
}

fn collect_stderr(handle: Option<thread::JoinHandle<Vec<u8>>>) -> String {
    match handle.and_then(|h| h.join().ok()) {
        Some(bytes) => String::from_utf8_lossy(&bytes).trim().to_string(),
        None => String::new(),
    }
}

fn bail_with_stderr(label: &str, status: std::process::ExitStatus, stderr: &str) -> Result<()> {
    if stderr.is_empty() {
        anyhow::bail!("{} exited with {}", label, status);
    }
    anyhow::bail!("{} exited with {}: {}", label, status, stderr);
}

/// Threshold above which `--input-files` is piped via stdin instead of argv (Windows cmdline
/// has a ~32k limit; assume average path ≈ 80 chars → 200 files is a safe cap).
const FILES_ON_ARGV_THRESHOLD: usize = 200;

/// Pass `files` either as repeated `--input-files <path>` args (small batches), or via
/// `--input-files-stdin` reading one path per line on stdin (large batches).
fn spawn_with_files(
    python: &Path,
    script: &Path,
    extra_args: &[&dyn AsRef<std::ffi::OsStr>],
    files: &[&Path],
) -> Result<Child> {
    let mut cmd = Command::new(python);
    cmd.arg("-u").arg(script);
    for a in extra_args {
        cmd.arg(a.as_ref());
    }

    let use_stdin = files.len() > FILES_ON_ARGV_THRESHOLD;
    if use_stdin {
        cmd.arg("--input-files-stdin");
        cmd.stdin(Stdio::piped());
    } else {
        cmd.arg("--input-files");
        for f in files {
            cmd.arg(f);
        }
    }

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn python {:?}", script))?;

    if use_stdin {
        if let Some(mut stdin) = child.stdin.take() {
            for f in files {
                writeln!(stdin, "{}", f.display())
                    .context("failed to write file path to stdin")?;
            }
        }
    }
    Ok(child)
}

/// Spawn `python summarize.py --input <input_dir>` and stream stdout to `on_line`.
///
/// `notes_dir` is forwarded as `--notes-dir` to summarize.py (override of `paths.notes_dir`).
/// `python -u` (unbuffered) ensures the caller receives lines as they are produced rather
/// than at process exit.
pub fn run_summarize(
    python: &Path,
    script: &Path,
    input_dir: &Path,
    notes_dir: Option<&Path>,
    on_line: &dyn Fn(&str),
) -> Result<()> {
    let mut cmd = Command::new(python);
    cmd.arg("-u")
        .arg(script)
        .arg("--input")
        .arg(input_dir);
    if let Some(notes) = notes_dir {
        cmd.arg("--notes-dir").arg(notes);
    }
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn python {:?}", script))?;

    let err_handle = drain_stderr(&mut child);

    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            on_line(&line);
        }
    }

    let status = child.wait().context("python process error")?;
    let stderr = collect_stderr(err_handle);
    if !status.success() {
        bail_with_stderr(&format!("python {:?}", script), status, &stderr)?;
    }
    Ok(())
}

/// Spawn `summarize.py --input-files <files>` and stream stdout to `on_line`.
///
/// Variant of [`run_summarize`] that takes an explicit list of files instead of an input
/// directory. Used by the "Organiser les notes" window to re-summarize a user-picked subset.
pub fn run_summarize_files(
    python: &Path,
    script: &Path,
    files: &[&Path],
    notes_dir: Option<&Path>,
    on_line: &dyn Fn(&str),
) -> Result<()> {
    let notes_arg = notes_dir.map(|p| p.to_path_buf());
    let mut extras: Vec<&dyn AsRef<std::ffi::OsStr>> = Vec::new();
    let notes_flag: &str = "--notes-dir";
    if let Some(n) = notes_arg.as_ref() {
        extras.push(&notes_flag);
        extras.push(n);
    }

    let mut child = spawn_with_files(python, script, &extras, files)?;
    let err_handle = drain_stderr(&mut child);

    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            on_line(&line);
        }
    }

    let status = child.wait().context("python process error")?;
    let stderr = collect_stderr(err_handle);
    if !status.success() {
        bail_with_stderr(&format!("python {:?}", script), status, &stderr)?;
    }
    Ok(())
}

/// Spawn `group_notes.py --input-files <files> --output <path>` and stream stdout to `on_line`.
///
/// Concatenates the given .md notes into a single grouped markdown file at `output`.
pub fn run_group(
    python: &Path,
    script: &Path,
    files: &[&Path],
    output: &Path,
    on_line: &dyn Fn(&str),
) -> Result<()> {
    let output_buf = output.to_path_buf();
    let output_flag: &str = "--output";
    let extras: Vec<&dyn AsRef<std::ffi::OsStr>> = vec![&output_flag, &output_buf];

    let mut child = spawn_with_files(python, script, &extras, files)?;
    let err_handle = drain_stderr(&mut child);

    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            on_line(&line);
        }
    }

    let status = child.wait().context("python process error")?;
    let stderr = collect_stderr(err_handle);
    if !status.success() {
        bail_with_stderr(&format!("python {:?}", script), status, &stderr)?;
    }
    Ok(())
}

/// Spawn `apply_template.py --template <t> --input-files <files> [--output <path>]`.
///
/// If `output` is `None`, the rendered text is captured from stdout and returned as a String.
/// If `output` is `Some(path)`, the script writes there and the returned String is empty.
pub fn run_apply_template(
    python: &Path,
    script: &Path,
    template: &Path,
    files: &[&Path],
    output: Option<&Path>,
) -> Result<String> {
    let template_buf = template.to_path_buf();
    let template_flag: &str = "--template";
    let output_flag: &str = "--output";
    let output_buf = output.map(|p| p.to_path_buf());

    let mut extras: Vec<&dyn AsRef<std::ffi::OsStr>> = vec![&template_flag, &template_buf];
    if let Some(out) = output_buf.as_ref() {
        extras.push(&output_flag);
        extras.push(out);
    }

    let child = spawn_with_files(python, script, &extras, files)?;
    let out = child
        .wait_with_output()
        .context("apply_template.py process error")?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("apply_template.py exited with {}: {}", out.status, stderr.trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// One NDJSON record from `classify.py --batch`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifyResult {
    pub file: String,
    pub suggested_path: String,
    pub confidence: f32,
    pub method: crate::sort_emails::ClassifyMethod,
}

/// Spawn `classify.py --batch` and stream NDJSON results to `on_result` as each line arrives.
///
/// Each line on the script's stdout is one JSON object (see `ClassifyResult`).
/// `on_result` is called per file, allowing the caller to push live updates to a UI
/// without waiting for the entire batch to finish.
pub fn run_classify_batch(
    python: &Path,
    script: &Path,
    data_dir: &Path,
    files: &[&Path],
    on_result: &dyn Fn(&ClassifyResult),
) -> Result<Vec<ClassifyResult>> {
    let data_dir_buf = data_dir.to_path_buf();
    let batch_flag: &str = "--batch";
    let data_dir_flag: &str = "--data-dir";
    let extras: Vec<&dyn AsRef<std::ffi::OsStr>> =
        vec![&batch_flag, &data_dir_flag, &data_dir_buf];

    let mut child = spawn_with_files(python, script, &extras, files)?;
    let err_handle = drain_stderr(&mut child);

    let mut results: Vec<ClassifyResult> = Vec::with_capacity(files.len());
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ClassifyResult>(&line) {
                Ok(r) => {
                    on_result(&r);
                    results.push(r);
                }
                Err(e) => {
                    eprintln!("classify NDJSON line ignored: {} ({})", line, e);
                }
            }
        }
    }

    let status = child.wait().context("classify.py process error")?;
    let stderr = collect_stderr(err_handle);
    if !status.success() {
        bail_with_stderr("classify.py", status, &stderr)?;
    }
    Ok(results)
}

/// Persist a batch of user-confirmed decisions in one Python subprocess call.
///
/// Spawns `classify.py --record-decisions-batch` once and pipes JSONL on stdin
/// (`{"file":"…","path":"…"}` per line). Avoids N spawns for N decisions.
pub fn record_decisions_batch(
    python: &Path,
    script: &Path,
    data_dir: &Path,
    decisions: &[(PathBuf, String)],
) -> Result<usize> {
    if decisions.is_empty() {
        return Ok(0);
    }

    let mut child = Command::new(python)
        .arg("-u")
        .arg(script)
        .arg("--record-decisions-batch")
        .arg("--data-dir")
        .arg(data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn python {:?}", script))?;

    let err_handle = drain_stderr(&mut child);

    if let Some(mut stdin) = child.stdin.take() {
        for (file, path) in decisions {
            let line = serde_json::json!({
                "file": file.to_string_lossy(),
                "path": path,
            });
            writeln!(stdin, "{}", line).context("failed to write decision line to stdin")?;
        }
        // dropping stdin closes the pipe → child sees EOF on stdin
    }

    let mut applied = 0usize;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                if let Some(n) = v.get("applied").and_then(|n| n.as_u64()) {
                    applied = n as usize;
                }
            }
        }
    }

    let status = child.wait().context("classify.py process error")?;
    let stderr = collect_stderr(err_handle);
    if !status.success() {
        bail_with_stderr("classify.py --record-decisions-batch", status, &stderr)?;
    }
    Ok(applied)
}
