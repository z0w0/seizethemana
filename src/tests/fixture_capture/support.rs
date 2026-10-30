//! Small process helpers for offline capture tests.

use std::path::Path;
use std::process::Command;

/// Locate checked-in commands and source data without depending on the test's cwd.
pub fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Write an executable command shim inside a test-owned directory.
pub fn write_stub(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path.parent().expect("a command shim has a parent"))
        .expect("create command shim directory");
    std::fs::write(path, contents).expect("write command shim");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("make command shim executable");
}

/// Run a retained capture command with isolated temporary files and command shims.
pub fn script_command(script: &str, work: &Path, bin: &Path) -> Command {
    let inherited = std::env::var_os("PATH").expect("capture tests require a command path");
    let path = std::env::join_paths(
        std::iter::once(bin.to_path_buf()).chain(std::env::split_paths(&inherited)),
    )
    .expect("join test command path");
    let mut command = Command::new("bash");
    command
        .arg(root().join("scripts").join(script))
        .current_dir(work)
        .env("TMPDIR", work)
        .env("PATH", path);
    command
}
