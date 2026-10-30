//! Streaming downloads with named progress and safe destination replacement.

use anyhow::Context;
use std::io::{Read, Write};
use std::path::Path;

/// Download one named file and retain a completion line only after replacement.
pub(crate) fn download_to_named(
    uri: &str,
    dest: &Path,
    compressed_size: Option<u64>,
    out: &mut crate::output::Output,
    label: &str,
) -> anyhow::Result<u64> {
    let start = std::time::Instant::now();
    out.start_download(label, compressed_size.unwrap_or(0));
    match download_file(uri, dest, compressed_size, out, label) {
        Ok(written) => {
            out.finish(
                "Downloaded",
                &format!("{label} ({})", crate::output::byte_count(written)),
                start.elapsed(),
            );
            Ok(written)
        }
        Err(err) => {
            out.clear_progress();
            Err(err.context(format!("downloading {label}")))
        }
    }
}

/// Replace the destination only after saving the complete response.
fn download_file(
    uri: &str,
    dest: &Path,
    compressed_size: Option<u64>,
    out: &mut crate::output::Output,
    label: &str,
) -> anyhow::Result<u64> {
    let client = super::http_client()?;
    let mut response = client.get(uri).send()?.error_for_status()?;
    let total = response.content_length().or(compressed_size);
    out.set_progress_total(total.unwrap_or(0));
    let tmp = dest.with_extension(format!(
        "{}.tmp",
        dest.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
    ));
    let mut file =
        std::fs::File::create(&tmp).with_context(|| format!("cannot create {}", tmp.display()))?;
    let result = write_response(&mut response, &mut file, &tmp, total, out, label);
    drop(file);
    let result = result.and_then(|written| {
        std::fs::rename(&tmp, dest)
            .with_context(|| format!("replacing {} with downloaded data", dest.display()))?;
        Ok(written)
    });
    match result {
        Ok(written) => Ok(written),
        Err(err) => {
            if let Err(cleanup_err) = std::fs::remove_file(&tmp) {
                return Err(err.context(format!(
                    "cannot remove incomplete download {}: {cleanup_err}",
                    tmp.display()
                )));
            }
            Err(err)
        }
    }
}

/// Count response bytes saved to disk and report plain-text checkpoints.
fn write_response(
    response: &mut reqwest::blocking::Response,
    file: &mut std::fs::File,
    tmp: &Path,
    total: Option<u64>,
    out: &mut crate::output::Output,
    label: &str,
) -> anyhow::Result<u64> {
    let plain_progress = !out.supports_progress();
    let known_total = total.filter(|total| *total > 0);
    let step = known_total
        .map(|total| total / 10)
        .unwrap_or(5 * 1024 * 1024)
        .max(1);
    let mut written = 0u64;
    let mut last_logged = 0u64;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let count = response.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        file.write_all(&chunk[..count])
            .with_context(|| format!("failed writing {}", tmp.display()))?;
        written += count as u64;
        if plain_progress {
            if written - last_logged >= step {
                last_logged = written;
                let bytes = match known_total {
                    Some(total) => format!(
                        "{}/{}",
                        crate::output::byte_count(written),
                        crate::output::byte_count(total)
                    ),
                    None => crate::output::byte_count(written),
                };
                out.status("Downloading", &format!("{label} ({bytes})"));
            }
        } else {
            out.set_progress_position(written);
        }
    }
    Ok(written)
}

/// Loopback checks for download results, progress copy, and failed replacement.
#[cfg(test)]
#[path = "tests/download_tests.rs"]
mod tests;
