# Fix code quality
fmt:
  cargo fmt --all || true
  cargo clippy --all-targets --fix --allow-dirty --allow-staged --all-features || true

docs-serve:
  uvx zensical serve

wasm:
  just -f crates/datalink-wasm/justfile

wasm-test:
  just -f crates/datalink-wasm/justfile test