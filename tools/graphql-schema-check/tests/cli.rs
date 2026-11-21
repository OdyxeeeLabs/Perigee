use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BASELINE: &str = "type Query { vault: Vault }\ntype Vault { id: ID! name: String }\n";
const ADDITIVE: &str =
    "type Query { vault: Vault newField: String }\ntype Vault { id: ID! name: String }\n";
const BREAKING: &str = "type Query { vault: Vault }\ntype Vault { id: ID! }\n";

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "graphql-schema-check-{label}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temp directory");
        Self(path)
    }

    fn file(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, contents).expect("write temp file");
        path
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_graphql-schema-tools"))
}

fn path_string(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

#[test]
fn check_exits_zero_for_additive_changes() {
    let dir = TempDir::new("additive");
    let baseline = dir.file("baseline.graphql", BASELINE);
    let proposed = dir.file("proposed.graphql", ADDITIVE);

    let output = tool()
        .args(["check", path_string(&baseline), path_string(&proposed)])
        .output()
        .expect("run check");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("no breaking changes"));
}

#[test]
fn check_exits_nonzero_with_actionable_diagnostics() {
    let dir = TempDir::new("breaking");
    let baseline = dir.file("baseline.graphql", BASELINE);
    let proposed = dir.file("proposed.graphql", BREAKING);

    let output = tool()
        .args(["check", path_string(&baseline), path_string(&proposed)])
        .output()
        .expect("run check");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("field-removed"), "{stdout}");
    assert!(stdout.contains("`Vault.name`"), "{stdout}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("1 breaking change(s)"));
}

#[test]
fn check_writes_github_step_summary() {
    let dir = TempDir::new("summary");
    let baseline = dir.file("baseline.graphql", BASELINE);
    let proposed = dir.file("proposed.graphql", BREAKING);
    let summary = dir.path("summary.md");

    let output = tool()
        .args([
            "check",
            path_string(&baseline),
            path_string(&proposed),
            "--summary",
            path_string(&summary),
        ])
        .output()
        .expect("run check");

    assert!(!output.status.success());
    let summary_text = fs::read_to_string(&summary).expect("read summary");
    assert!(summary_text.contains("### GraphQL schema compatibility"));
    assert!(summary_text.contains("field-removed"));
}

#[test]
fn update_requires_an_explicit_reason_and_records_it() {
    let dir = TempDir::new("update");
    let baseline = dir.file("baseline.graphql", BASELINE);
    let proposed = dir.file("proposed.graphql", BREAKING);

    let rejected = tool()
        .args(["update", path_string(&baseline), path_string(&proposed)])
        .output()
        .expect("run update without reason");
    assert!(!rejected.status.success());
    assert_eq!(
        fs::read_to_string(&baseline).expect("read baseline"),
        BASELINE
    );

    let approved = tool()
        .args([
            "update",
            path_string(&baseline),
            path_string(&proposed),
            "--reason",
            "clients migrated in #123",
        ])
        .output()
        .expect("run update with reason");
    assert!(
        approved.status.success(),
        "{}",
        String::from_utf8_lossy(&approved.stderr)
    );

    let updated = fs::read_to_string(&baseline).expect("read updated baseline");
    assert!(updated.contains("# GraphQL schema baseline approved with: clients migrated in #123"));
    assert!(updated.contains("type Vault { id: ID! }"));

    let check = tool()
        .args(["check", path_string(&baseline), path_string(&proposed)])
        .output()
        .expect("recheck after update");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
}
