//! `cargo run -p lorca-mobile --features bindgen --bin uniffi-bindgen -- generate --library
//! target/debug/liblorca_mobile.dylib --language swift --out-dir …`
fn main() {
    uniffi::uniffi_bindgen_main()
}
