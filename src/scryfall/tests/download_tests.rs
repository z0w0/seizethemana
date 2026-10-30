//! Exercise download output and file replacement through loopback HTTP.

use super::*;
use crate::output::{Output, byte_count};
use std::net::{TcpListener, TcpStream};
use std::process::Stdio;
use std::time::{Duration, Instant};

/// Serve one bounded loopback request without an external HTTP dependency.
struct OneShotServer {
    uri: String,
    worker: std::thread::JoinHandle<()>,
}

impl OneShotServer {
    /// Return a fixed HTTP response after accepting the downloader's request.
    fn respond(response: Vec<u8>) -> Self {
        Self::with_handler(move |mut stream| {
            stream.write_all(&response).expect("write HTTP response");
        })
    }

    /// Allow tests to change temporary files while a response is in flight.
    fn with_handler(handler: impl FnOnce(TcpStream) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback server");
        listener
            .set_nonblocking(true)
            .expect("bound loopback accept time");
        let address = listener.local_addr().expect("loopback address");
        let worker = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "download made no HTTP request");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(err) => panic!("accept loopback request: {err}"),
                }
            };
            stream
                .set_nonblocking(false)
                .expect("use blocking request and response I/O");
            stream
                .set_read_timeout(Some(Duration::from_secs(15)))
                .expect("bound request reads");
            stream
                .set_write_timeout(Some(Duration::from_secs(15)))
                .expect("bound response writes");
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).expect("read HTTP request");
                assert!(count > 0, "request closed before its headers");
                request.extend_from_slice(&buffer[..count]);
            }
            handler(stream);
        });
        Self {
            uri: format!("http://{address}/bulk.gz"),
            worker,
        }
    }

    /// Surface server-side failures after the child has finished downloading.
    fn finish(self) {
        self.worker.join().expect("loopback HTTP server");
    }
}

/// Capture real task output in a child without touching the user's store.
fn run_worker(
    dest: &Path,
    uri: &str,
    compressed_size: Option<u64>,
    json: bool,
    mode: &str,
) -> std::process::Output {
    let mut command = std::process::Command::new(std::env::current_exe().expect("test executable"));
    command
        .args([
            "--exact",
            "scryfall::download::tests::download_output_worker",
            "--nocapture",
        ])
        .env("STM_TEST_DOWNLOAD_DEST", dest)
        .env("STM_TEST_DOWNLOAD_URI", uri)
        .env("STM_TEST_DOWNLOAD_MODE", mode)
        .env("STM_TEST_DOWNLOAD_JSON", if json { "1" } else { "0" })
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .stdin(Stdio::null());
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env_remove(name);
    }
    match compressed_size {
        Some(size) => {
            command.env("STM_TEST_DOWNLOAD_SIZE", size.to_string());
        }
        None => {
            command.env_remove("STM_TEST_DOWNLOAD_SIZE");
        }
    }
    let output = command.output().expect("run download worker");
    assert!(
        output.status.success(),
        "worker failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Build an ordinary close-delimited or known-length successful response.
fn response(body: &[u8], content_length: Option<u64>) -> Vec<u8> {
    let mut response = b"HTTP/1.1 200 OK\r\nConnection: close\r\n".to_vec();
    if let Some(length) = content_length {
        write!(response, "Content-Length: {length}\r\n").expect("write response length");
    }
    response.extend_from_slice(b"\r\n");
    response.extend_from_slice(body);
    response
}

/// Read captured human diagnostics without assuming terminal control codes.
fn stderr(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("UTF-8 download output")
}

/// Capture production status text from a subprocess.
#[test]
fn download_output_worker() {
    let Some(dest) = std::env::var_os("STM_TEST_DOWNLOAD_DEST") else {
        return;
    };
    let dest = std::path::PathBuf::from(dest);
    let uri = std::env::var("STM_TEST_DOWNLOAD_URI").expect("worker download URI");
    let mode = std::env::var("STM_TEST_DOWNLOAD_MODE").expect("worker mode");
    let compressed_size = std::env::var("STM_TEST_DOWNLOAD_SIZE")
        .ok()
        .map(|size| size.parse().expect("worker size"));
    let json = std::env::var("STM_TEST_DOWNLOAD_JSON").expect("worker output mode") == "1";
    let mut out = Output::new(json, true, false);
    let result = match mode.as_str() {
        "download" => download_to_named(&uri, &dest, compressed_size, &mut out, "Scryfall cards"),
        "bulk" => crate::scryfall::ensure_fresh_bulk_named(
            &crate::scryfall::BulkFile {
                uri,
                compressed_size,
                updated_at: None,
            },
            &dest,
            &mut out,
            "Scryfall tags",
        )
        .map(u64::from),
        "spellbook-cache" => {
            crate::spellbook::ensure_fresh_variants(&dest, &mut out, false).map(u64::from)
        }
        "spellbook-ingest" => {
            let mut conn = crate::db::open(&dest.with_extension("sqlite")).expect("test combo DB");
            crate::spellbook::ingest(&mut conn, &dest, &mut out, "now").map(|count| count as u64)
        }
        other => panic!("unknown worker mode: {other}"),
    };
    assert!(
        out.task_output().is_some(),
        "task left an active progress bar"
    );
    match result {
        Ok(value) => println!("result={value}"),
        Err(err) => {
            out.error(&format!("{err:#}"));
            println!("result=error");
        }
    }
}

/// A completed transfer reports its object, actual bytes, and elapsed time.
#[test]
fn known_length_reports_completion_after_replacing_the_destination() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    std::fs::write(&dest, b"old data").expect("seed old download");
    let body = vec![b'x'; 128 * 1024];
    let server = OneShotServer::respond(response(&body, Some(body.len() as u64)));
    let output = run_worker(&dest, &server.uri, Some(1), false, "download");
    server.finish();
    assert_eq!(std::fs::read(&dest).expect("completed download"), body);
    assert!(!dest.with_extension("gz.tmp").exists());
    let text = stderr(&output);
    assert!(text.contains("Downloading Scryfall cards"), "{text}");
    assert!(
        text.contains(&format!("/{})", byte_count(body.len() as u64))),
        "response length must override the index hint: {text}"
    );
    assert!(
        text.contains(&format!(
            "Downloaded Scryfall cards ({}) in ",
            byte_count(body.len() as u64)
        )),
        "{text}"
    );
    assert_eq!(text.matches("Downloaded Scryfall cards").count(), 1);
}

/// Unknown sizes keep active copy until the final result and never show /0.
#[test]
fn unknown_length_keeps_the_object_on_plain_checkpoints() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    let body = vec![b'x'; 5 * 1024 * 1024 + 1];
    for hint in [None, Some(0)] {
        let server = OneShotServer::respond(response(&body, None));
        let output = run_worker(&dest, &server.uri, hint, false, "download");
        server.finish();
        assert_eq!(std::fs::read(&dest).expect("completed download"), body);
        let text = stderr(&output);
        assert!(text.contains("Downloading Scryfall cards ("), "{text}");
        assert!(!text.contains("/0"), "{text}");
        assert_eq!(text.matches("Downloaded Scryfall cards").count(), 1);
        assert!(
            text.contains(&format!(
                "Downloaded Scryfall cards ({})",
                byte_count(body.len() as u64)
            )),
            "{text}"
        );
    }
}

/// Missing bulk files use the named downloader and return the download signal.
#[test]
fn missing_bulk_download_names_its_source_and_returns_true() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    let server = OneShotServer::respond(response(b"new tags", Some(8)));
    let output = run_worker(&dest, &server.uri, None, false, "bulk");
    server.finish();
    let text = stderr(&output);
    assert!(text.contains("Downloading Scryfall tags"), "{text}");
    assert!(text.contains("Downloaded Scryfall tags"), "{text}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("result=1"));
    assert_eq!(std::fs::read(&dest).expect("downloaded tags"), b"new tags");
}

/// A length hint does not replace the actual byte count in the final result.
#[test]
fn completion_uses_written_bytes_not_the_index_hint() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    let server = OneShotServer::respond(response(b"new data", None));
    let output = run_worker(&dest, &server.uri, Some(1_000_000), false, "download");
    server.finish();
    assert!(stderr(&output).contains(&format!("Downloaded Scryfall cards ({})", byte_count(8))));
    assert!(String::from_utf8_lossy(&output.stdout).contains("result=8"));
}

/// Empty responses complete without inventing a total or a nonzero byte count.
#[test]
fn zero_length_completes_without_a_zero_denominator() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    let server = OneShotServer::respond(response(b"", Some(0)));
    let output = run_worker(&dest, &server.uri, Some(123), false, "download");
    server.finish();
    assert_eq!(std::fs::metadata(&dest).expect("empty download").len(), 0);
    let text = stderr(&output);
    assert!(!text.contains("/0"), "{text}");
    assert!(
        text.contains(&format!("Downloaded Scryfall cards ({})", byte_count(0))),
        "{text}"
    );
}

/// HTTP failures preserve the old file and never claim a completed download.
#[test]
fn http_error_preserves_the_destination_without_a_completion() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    std::fs::write(&dest, b"old data").expect("seed old download");
    let server = OneShotServer::respond(
        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec(),
    );
    let output = run_worker(&dest, &server.uri, None, false, "download");
    server.finish();
    assert_eq!(std::fs::read(&dest).expect("old download"), b"old data");
    assert!(!dest.with_extension("gz.tmp").exists());
    let text = stderr(&output);
    assert!(text.contains("downloading Scryfall cards"), "{text}");
    assert!(text.contains("503"), "{text}");
    assert!(!text.contains("Downloaded"), "{text}");
}

/// An interrupted body removes partial data but leaves the old file intact.
#[test]
fn truncated_body_cleans_the_temporary_file() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    std::fs::write(&dest, b"old data").expect("seed old download");
    let server = OneShotServer::respond(response(b"short", Some(100)));
    let output = run_worker(&dest, &server.uri, None, false, "download");
    server.finish();
    assert_eq!(std::fs::read(&dest).expect("old download"), b"old data");
    assert!(!dest.with_extension("gz.tmp").exists());
    let text = stderr(&output);
    assert!(text.contains("error: downloading Scryfall cards"), "{text}");
    assert!(!text.contains("Downloaded"), "{text}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("result=error"));
}

/// Failed replacement removes the completed temporary file, not the destination.
#[test]
fn rename_error_cleans_the_download_without_a_completion() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    std::fs::create_dir(&dest).expect("block destination replacement");
    std::fs::write(dest.join("keep"), b"old data").expect("seed destination directory");
    let server = OneShotServer::respond(response(b"new data", Some(8)));
    let output = run_worker(&dest, &server.uri, None, false, "download");
    server.finish();
    assert_eq!(
        std::fs::read(dest.join("keep")).expect("old data"),
        b"old data"
    );
    assert!(!dest.with_extension("gz.tmp").exists());
    let text = stderr(&output);
    assert!(text.contains("replacing"), "{text}");
    assert!(!text.contains("Downloaded"), "{text}");
}

/// A failed temporary-file open does not remove a path the download never owned.
#[test]
fn create_error_preserves_the_existing_temporary_directory() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    std::fs::write(&dest, b"old data").expect("seed old download");
    let tmp = dest.with_extension("gz.tmp");
    std::fs::create_dir(&tmp).expect("block temporary file creation");
    let server = OneShotServer::respond(response(b"new data", Some(8)));
    let output = run_worker(&dest, &server.uri, None, false, "download");
    server.finish();
    assert!(tmp.is_dir());
    assert_eq!(std::fs::read(&dest).expect("old download"), b"old data");
    let text = stderr(&output);
    assert!(text.contains("cannot create"), "{text}");
    assert!(!text.contains("Downloaded"), "{text}");
}

/// Keep both causes when another actor prevents removal of an incomplete file.
#[cfg(unix)]
#[test]
fn cleanup_error_keeps_the_transfer_error_in_its_chain() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    std::fs::write(&dest, b"old data").expect("seed old download");
    let tmp = dest.with_extension("gz.tmp");
    let server_tmp = tmp.clone();
    let server = OneShotServer::with_handler(move |mut stream| {
        stream
            .write_all(&response(b"short", Some(100)))
            .expect("send partial response");
        let deadline = Instant::now() + Duration::from_secs(15);
        while !server_tmp.exists() {
            assert!(
                Instant::now() < deadline,
                "download created no temporary file"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        std::fs::remove_file(&server_tmp).expect("unlink open temporary file");
        std::fs::create_dir(&server_tmp).expect("prevent incomplete-file cleanup");
    });
    let output = run_worker(&dest, &server.uri, None, false, "download");
    server.finish();
    assert_eq!(std::fs::read(&dest).expect("old download"), b"old data");
    assert!(tmp.is_dir());
    let text = stderr(&output);
    assert!(text.contains("cannot remove incomplete download"), "{text}");
    assert!(text.contains("downloading Scryfall cards"), "{text}");
    assert!(!text.contains("Downloaded"), "{text}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("result=error"));
}

/// Failures before the HTTP request still clear task state and name the object.
#[test]
fn invalid_uri_reports_the_download_object() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    let output = run_worker(&dest, "not a URI", None, false, "download");
    assert!(!dest.exists());
    let text = stderr(&output);
    assert!(text.contains("downloading Scryfall cards"), "{text}");
    assert!(!text.contains("Downloaded"), "{text}");
}

/// Successful JSON-mode downloads do not leak human status or progress text.
#[test]
fn json_mode_keeps_download_diagnostics_silent() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    let server = OneShotServer::respond(response(b"new data", Some(8)));
    let output = run_worker(&dest, &server.uri, None, true, "download");
    server.finish();
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
    assert_eq!(
        std::fs::read(&dest).expect("completed download"),
        b"new data"
    );
}

/// Fresh bulk and combo files identify their source without making a request.
#[test]
fn cache_hits_name_the_reused_object() {
    let temp = tempfile::tempdir().expect("download directory");
    let dest = temp.path().join("bulk.gz");
    std::fs::write(&dest, b"cached data").expect("seed fresh download");
    for (mode, label) in [
        ("bulk", "Scryfall tags"),
        ("spellbook-cache", "Commander Spellbook combos"),
    ] {
        let output = run_worker(&dest, "not a URI", None, false, mode);
        let text = stderr(&output);
        assert!(
            text.contains(&format!("Reusing {label} (cached)")),
            "{text}"
        );
        assert!(!text.contains("Downloading"), "{text}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("result=0"));
    }
}

/// Combo ingest keeps one object label through parsing and committed results.
#[test]
fn spellbook_ingest_reports_its_own_start_and_result() {
    let temp = tempfile::tempdir().expect("combo directory");
    let dest = temp.path().join("variants.json.gz");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder
        .write_all(b"{\"variants\":[]}")
        .expect("write empty combo fixture");
    std::fs::write(&dest, encoder.finish().expect("finish combo gzip"))
        .expect("write combo fixture");
    let output = run_worker(&dest, "not a URI", None, false, "spellbook-ingest");
    let text = stderr(&output);
    assert!(
        text.contains("Loading Commander Spellbook combos"),
        "{text}"
    );
    assert!(
        text.contains("Loaded Commander Spellbook combos (0 combos) in "),
        "{text}"
    );
}

/// A malformed combo file clears loading progress without reporting success.
#[test]
fn spellbook_ingest_failure_does_not_report_loaded() {
    let temp = tempfile::tempdir().expect("combo directory");
    let dest = temp.path().join("variants.json.gz");
    std::fs::write(&dest, b"not gzip").expect("write malformed combo fixture");
    let output = run_worker(&dest, "not a URI", None, false, "spellbook-ingest");
    let text = stderr(&output);
    assert!(
        text.contains("Loading Commander Spellbook combos"),
        "{text}"
    );
    assert!(
        !text.contains("Loaded Commander Spellbook combos"),
        "{text}"
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("result=error"));
}
