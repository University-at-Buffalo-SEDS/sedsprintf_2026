mod c_system_test {
    use std::path::PathBuf;
    use std::process::Command;

    fn run_exe(root: &PathBuf, name: &str) {
        let exe = root.join("build").join(name);
        let output = Command::new(&exe)
            .current_dir(root)
            .output()
            .unwrap_or_else(|e| panic!("Failed to run {name}: {e}"));

        eprintln!("{} stdout:\n{}", name, String::from_utf8_lossy(&output.stdout));
        eprintln!("{} stderr:\n{}", name, String::from_utf8_lossy(&output.stderr));

        assert!(
            output.status.success(),
            "{} failed with exit code {:?}",
            name,
            output.status.code()
        );
    }

    #[test]
    fn run_c_system_test() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("c-system-test");
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        // Force-refresh Rust staticlib in non-python mode so C linking never
        // picks up stale pyo3-enabled artifacts from unrelated local builds.
        let status = Command::new("python3")
            .arg("build.py")
            .arg("timesync")
            .arg("device_id=SYSTEM_TEST")
            .current_dir(&repo_root)
            .status()
            .expect("Failed to run build.py for c-system-test");
        assert!(status.success(), "build.py prebuild failed");

        let status = Command::new("cmake")
            .arg("-S")
            .arg(".")
            .arg("-B")
            .arg("build")
            .arg("-DCMAKE_BUILD_TYPE=Debug")
            .current_dir(&root)
            .status()
            .expect("Failed to config cmake build");
        assert!(status.success(), "CMake config failed");

        // Build the C project
        let status = Command::new("cmake")
            .arg("--build")
            .arg("build")
            .current_dir(&root)
            .status()
            .expect("Failed to invoke cmake build");
        assert!(status.success(), "CMake build failed");

        run_exe(&root, "c_system_test");
        run_exe(&root, "c_system_timesync_test");
        run_exe(&root, "c_system_board_topology_timesync_test");
    }
}
