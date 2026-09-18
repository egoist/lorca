//! `cargo run -p lorca-markdown --features bindgen --bin uniffi-bindgen -- generate --library
//! target/debug/liblorca_markdown.dylib --language swift --out-dir …`
fn main() {
    uniffi::uniffi_bindgen_main()
}
