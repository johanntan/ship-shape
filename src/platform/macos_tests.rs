//! Exercise the actual helper script with disposable directories. Only launch/report commands
//! and fault injection are substituted, so tests cannot open apps or display desktop alerts.
use std::{
	fs,
	path::PathBuf,
	process::{Child, Command},
	thread,
	time::{Duration, Instant},
};

use tempfile::TempDir;

use super::{PreparedUpdate, bundle_stamp, check_cancelled, find_replacement, validate_bundle};

struct Fixture {
	root: TempDir,
	bundle: PathBuf,
	stage: PathBuf,
}

impl Fixture {
	fn new() -> Self {
		let root = tempfile::Builder::new().prefix("ship-shape-O'Brien test-").tempdir().unwrap();
		let bundle = root.path().join("Current App.app");
		let stage = root.path().join("stage");
		fs::create_dir(&bundle).unwrap();
		fs::create_dir(&stage).unwrap();
		super::write_messages(&stage).unwrap();
		fs::create_dir(stage.join("new.app")).unwrap();
		fs::write(bundle.join("version"), "old").unwrap();
		fs::write(stage.join("new.app/version"), "new").unwrap();
		Self { root, bundle, stage }
	}

	fn launch(&self, pid: u32, commit: bool, fault: &str) -> Child {
		let mut script = include_str!("macos_install.sh").to_string();
		// Replace desktop alerts with a successful noninteractive command.
		script = script.replace("/usr/bin/osascript -", "/usr/bin/true");
		let launcher = self.root.path().join("launch.sh");
		fs::write(
			&launcher,
			if fault == "launch" {
				"exit 1\n"
			} else {
				"printf '%s\\n' \"$@\" > \"$(dirname \"$0\")/launch-args\"\nexit 0\n"
			},
		)
		.unwrap();
		script = script.replace("/usr/bin/open -n", &format!("/bin/sh {}", quote(launcher.to_str().unwrap())));
		if fault == "rename" || fault == "restore" {
			let renamer = self.root.path().join("rename.sh");
			let count = self.root.path().join("renames");
			let restore = if fault == "restore" { " -o \"$n\" -eq 3" } else { "" };
			fs::write(&renamer, format!("count={}\nn=0\n[ ! -f \"$count\" ] || n=$(cat \"$count\")\nn=$((n+1))\nprintf '%s' \"$n\" > \"$count\"\n[ \"$n\" -ne 2{restore} ] || exit 1\n/usr/bin/perl -e 'rename $ARGV[0], $ARGV[1] or die $!' \"$1\" \"$2\"\n", quote(count.to_str().unwrap()))).unwrap();
			// Use an explicit condition for the restore failure (both calls 2 and 3 fail).
			if fault == "restore" {
				let contents = fs::read_to_string(&renamer)
					.unwrap()
					.replace("[ \"$n\" -ne 2 -o \"$n\" -eq 3 ]", "[ \"$n\" -eq 1 ]");
				fs::write(&renamer, contents).unwrap();
			}
			script = script.replace(
				"/usr/bin/perl -e 'rename $ARGV[0], $ARGV[1] or die \"rename: $!\\n\"'",
				&format!("/bin/sh {}", quote(renamer.to_str().unwrap())),
			);
		}
		if fault == "timeout" {
			script = script.replace("-ge 300", "-ge 3").replace("/bin/sleep 0.2", "/bin/sleep 0.01");
		}
		let stamp = bundle_stamp(&self.bundle).unwrap();
		if fault == "changed" {
			fs::rename(&self.bundle, self.root.path().join("another-installer-backup.app")).unwrap();
			fs::create_dir(&self.bundle).unwrap();
			fs::write(self.bundle.join("version"), "other").unwrap();
		}
		let script_path = self.root.path().join("helper.sh");
		fs::write(&script_path, script).unwrap();
		if commit {
			fs::write(self.stage.join("commit"), "yes").unwrap();
		}
		Command::new("/bin/sh")
			.arg(script_path)
			.arg(pid.to_string())
			.arg(&self.bundle)
			.arg(&self.stage)
			.arg(self.root.path().join("log"))
			.arg(stamp)
			.arg("--env")
			.arg("TEST_CONFIG_PATH=O'Brien test/settings")
			.spawn()
			.unwrap()
	}

	fn version(&self) -> String {
		fs::read_to_string(self.bundle.join("version")).unwrap()
	}
}

fn quote(path: &str) -> String {
	format!("'{}'", path.replace('\'', "'\\''"))
}

fn wait(mut child: Child) -> bool {
	let deadline = Instant::now() + Duration::from_secs(5);
	loop {
		if let Some(status) = child.try_wait().unwrap() {
			return status.success();
		}
		if Instant::now() > deadline {
			child.kill().unwrap();
			child.wait().unwrap();
			panic!("helper did not exit");
		}
		thread::sleep(Duration::from_millis(10));
	}
}

#[test]
fn helper_replaces_and_cleans_up_with_quoted_paths() {
	let fixture = Fixture::new();
	assert!(wait(fixture.launch(u32::MAX, true, "")));
	assert_eq!(fixture.version(), "new");
	assert!(!fixture.stage.exists());
	let args = fs::read_to_string(fixture.root.path().join("launch-args")).unwrap();
	assert!(args.ends_with("--env\nTEST_CONFIG_PATH=O'Brien test/settings\n"));
}

#[test]
fn helper_restores_old_bundle_after_install_failure() {
	let fixture = Fixture::new();
	assert!(!wait(fixture.launch(u32::MAX, true, "rename")));
	assert_eq!(fixture.version(), "old");
	assert!(!fixture.stage.exists());
}

#[test]
fn helper_preserves_backup_when_restore_fails() {
	let fixture = Fixture::new();
	assert!(!wait(fixture.launch(u32::MAX, true, "restore")));
	assert_eq!(fs::read_to_string(fixture.stage.join("old.app/version")).unwrap(), "old");
	assert_eq!(fs::read_to_string(fixture.stage.join("new.app/version")).unwrap(), "new");
}

#[test]
fn helper_preserves_backup_when_launch_fails() {
	let fixture = Fixture::new();
	assert!(!wait(fixture.launch(u32::MAX, true, "launch")));
	assert_eq!(fixture.version(), "new");
	assert_eq!(fs::read_to_string(fixture.stage.join("old.app/version")).unwrap(), "old");
}

#[test]
fn helper_does_not_replace_an_installation_changed_by_another_installer() {
	let fixture = Fixture::new();
	assert!(!wait(fixture.launch(u32::MAX, true, "changed")));
	assert_eq!(fixture.version(), "other");
}

#[test]
fn helper_times_out_without_commit_or_host_exit() {
	for commit in [false, true] {
		let fixture = Fixture::new();
		assert!(!wait(fixture.launch(std::process::id(), commit, "timeout")));
		assert_eq!(fixture.version(), "old");
		assert!(!fixture.stage.exists());
	}
}

#[test]
fn dropping_prepared_update_cancels_before_replacement() {
	let fixture = Fixture::new();
	let child = fixture.launch(u32::MAX, false, "");
	let deadline = Instant::now() + Duration::from_secs(2);
	while !fixture.stage.join("ready").exists() {
		assert!(Instant::now() < deadline);
		thread::sleep(Duration::from_millis(10));
	}
	fs::remove_dir_all(&fixture.stage).unwrap();
	assert!(wait(child));
	assert_eq!(fixture.version(), "old");
	let stage = tempfile::tempdir().unwrap();
	let path = stage.path().to_owned();
	drop(PreparedUpdate { stage });
	assert!(!path.exists());
}

fn metadata_bundle(root: &std::path::Path, name: &str, identity: &str, executable: &str) -> PathBuf {
	let bundle = root.join(name);
	fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
	fs::write(bundle.join("Contents/Info.plist"), format!("<plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{identity}</string><key>CFBundleExecutable</key><string>{executable}</string></dict></plist>")).unwrap();
	bundle
}

#[test]
fn replacement_requires_exactly_one_matching_bundle_and_ignores_symlinks() {
	let root = tempfile::tempdir().unwrap();
	assert!(find_replacement(root.path(), "correct.id").is_err());
	metadata_bundle(root.path(), "Wrong.app", "wrong.id", "app");
	let first = metadata_bundle(root.path(), "First.app", "correct.id", "app");
	std::os::unix::fs::symlink(&first, root.path().join("Link.app")).unwrap();
	assert_eq!(find_replacement(root.path(), "correct.id").unwrap(), first);
	metadata_bundle(root.path(), "Duplicate.app", "correct.id", "app");
	assert!(find_replacement(root.path(), "correct.id").is_err());
}

#[test]
fn rejects_missing_escaping_and_unsigned_executables() {
	use std::os::unix::fs::PermissionsExt;
	let root = tempfile::tempdir().unwrap();
	let bundle = metadata_bundle(root.path(), "Missing.app", "id", "app");
	assert!(validate_bundle(&bundle).is_err());
	let bundle = metadata_bundle(root.path(), "Escaping.app", "id", "../../outside");
	assert!(validate_bundle(&bundle).is_err());
	let bundle = metadata_bundle(root.path(), "Unsigned.app", "id", "app");
	let executable = bundle.join("Contents/MacOS/app");
	fs::write(&executable, "#!/bin/sh\nexit 0\n").unwrap();
	fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
	assert!(validate_bundle(&bundle).is_err());
}

#[test]
fn cancellation_stops_preparation() {
	assert!(check_cancelled(&std::sync::atomic::AtomicBool::new(true)).is_err());
}

#[test]
fn protected_and_symlinked_bundles_use_manual_fallback() {
	use std::os::unix::fs::PermissionsExt;
	let root = tempfile::tempdir().unwrap();
	let bundle = metadata_bundle(root.path(), "Protected.app", "id", "app");
	fs::set_permissions(&bundle, fs::Permissions::from_mode(0o555)).unwrap();
	assert!(!super::can_update(&bundle));
	fs::set_permissions(&bundle, fs::Permissions::from_mode(0o755)).unwrap();
	let link = root.path().join("Link.app");
	std::os::unix::fs::symlink(&bundle, &link).unwrap();
	assert!(!super::can_update(&link));
}

#[test]
fn signed_bundle_passes_but_tampering_fails() {
	let root = tempfile::tempdir().unwrap();
	let bundle = metadata_bundle(root.path(), "Signed.app", "id", "app");
	fs::create_dir(bundle.join("Contents/Resources")).unwrap();
	let resource = bundle.join("Contents/Resources/content.txt");
	fs::write(&resource, "original").unwrap();
	// A Mach-O executable, copied into a disposable fixture, avoids signing shell scripts.
	fs::copy("/usr/bin/true", bundle.join("Contents/MacOS/app")).unwrap();
	let status = Command::new("/usr/bin/codesign").args(["--force", "--sign", "-"]).arg(&bundle).output().unwrap();
	assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
	validate_bundle(&bundle).unwrap();
	fs::write(&resource, "tampered").unwrap();
	let error = validate_bundle(&bundle).unwrap_err();
	assert!(error.contains("signature verification failed"), "{error}");
}
