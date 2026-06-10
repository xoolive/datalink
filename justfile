# Fix code quality (ruff, rustfmt)
fmt:
  cargo fmt --all || true
  cargo clippy --all-targets --fix --allow-dirty --allow-staged --all-features || true
