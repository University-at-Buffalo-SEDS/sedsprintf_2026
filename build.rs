fn main() {
    if std::env::var_os("CARGO_FEATURE_PYTHON").is_some() {
        // Compile as a Python extension module only when the `python` feature is enabled
        println!("cargo:rustc-crate-type=cdylib");
    }
}
