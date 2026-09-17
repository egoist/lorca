//! `cargo run -p tinybot-mobile --features bindgen --bin uniffi-bindgen -- generate --library
//! target/debug/libtinybot_mobile.dylib --language swift --out-dir …`
fn main() {
    uniffi::uniffi_bindgen_main()
}
