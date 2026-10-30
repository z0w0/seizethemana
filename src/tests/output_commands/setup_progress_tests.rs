//! Check embedding checkpoints with production-sized chunks and no model.

use super::*;

/// Large and partial runs report completed chunks without logging each card.
#[test]
fn embedding_checkpoints_are_bounded_and_include_the_last_card() {
    for total in [1, EMBED_CHUNK, 1_000, 32_572] {
        let mut last_reported = 0;
        let mut checkpoints = Vec::new();
        for start in (0..total).step_by(EMBED_CHUNK) {
            let completed = (start + EMBED_CHUNK).min(total);
            if should_report_embed_checkpoint(total, completed, last_reported) {
                last_reported = completed;
                checkpoints.push(completed);
            }
        }
        assert_eq!(checkpoints.last(), Some(&total));
        assert!(checkpoints.len() <= 11, "{total} cards: {checkpoints:?}");
        assert!(checkpoints.windows(2).all(|pair| pair[0] < pair[1]));
    }
    assert!(!should_report_embed_checkpoint(0, 0, 0));
    assert!(!should_report_embed_checkpoint(32_572, EMBED_CHUNK, 0));
}

/// A small encoder keeps setup output checks independent of model downloads.
struct FixtureEmbedder {
    /// Fail during embedding instead of producing index rows.
    fail: bool,
}

impl crate::embed::DocumentEmbedder for FixtureEmbedder {
    fn embed_documents(&mut self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        anyhow::ensure!(!self.fail, "synthetic embedding failure");
        Ok(texts.iter().map(|_| vec![1.0; crate::embed::DIM]).collect())
    }
}

/// Run the real index-building path with an offline encoder and isolated data.
#[test]
fn index_progress_worker() {
    let Some(root) = std::env::var_os("STM_SETUP_PROGRESS_ROOT") else {
        return;
    };
    let mode = std::env::var("STM_SETUP_PROGRESS_MODE").expect("setup fixture mode");
    let paths = Paths::new(root.into());
    paths.ensure_dirs().expect("fixture directories");
    let conn = crate::db::open(&paths.db()).expect("fixture database");
    conn.execute(
        "INSERT INTO cards (name, oracle_id) VALUES ('One', 'one'), ('Two', 'two')",
        [],
    )
    .expect("fixture cards");
    let mut out = Output::new(false, true, false);
    let result = embed_all_with_model(&paths, &mut out, &conn, || {
        anyhow::ensure!(mode != "load-error", "synthetic model failure");
        Ok(Box::new(FixtureEmbedder {
            fail: mode == "embed-error",
        }))
    });
    if mode == "success" {
        assert_eq!(result.expect("completed fixture index").len(), 2);
        assert!(paths.vectors_file().exists());
    } else {
        let error = result.expect_err("fixture failure");
        out.error(&error.to_string());
        assert!(!paths.vectors_file().exists());
    }
}

/// Even a fast index build leaves a named result; failed work cannot claim completion.
#[test]
fn index_progress_preserves_results_and_clears_failed_steps() {
    for mode in ["success", "load-error", "embed-error"] {
        let temp = tempfile::tempdir().expect("setup fixture store");
        let result = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "setup::setup_progress_tests::index_progress_worker",
                "--nocapture",
            ])
            .env("STM_SETUP_PROGRESS_ROOT", temp.path())
            .env("STM_SETUP_PROGRESS_MODE", mode)
            .output()
            .expect("index output worker");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let text = String::from_utf8(result.stderr).expect("setup stderr");
        assert!(
            text.contains("Loading search model (downloads files if needed)"),
            "{text}"
        );
        assert!(!text.contains('\u{1b}'));
        if mode == "load-error" {
            assert!(!text.contains("Loaded search model"), "{text}");
            assert!(text.contains("error: synthetic model failure"), "{text}");
        } else {
            assert!(text.contains("Loaded search model in"), "{text}");
            assert!(text.contains("Building card search index"), "{text}");
        }
        if mode == "success" {
            assert!(
                text.contains("Built card search index (2 cards) in"),
                "{text}"
            );
        } else {
            assert!(!text.contains("Built card search index"), "{text}");
        }
    }
}
