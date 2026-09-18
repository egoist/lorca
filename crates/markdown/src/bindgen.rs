//! `cargo run -p tinybot-markdown --features bindgen --bin uniffi-bindgen -- generate --library
//! target/debug/libtinybot_markdown.dylib --language swift --out-dir …`
fn main() {
    uniffi::uniffi_bindgen_main()
}
