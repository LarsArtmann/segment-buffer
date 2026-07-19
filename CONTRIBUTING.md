# Contributing

Thanks for your interest in contributing to segment-buffer!

## How to Contribute

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Submit a pull request

## Development Setup

```bash
# Run all tests
cargo test --no-fail-fast --features encryption

# Lint
cargo clippy --all-targets --features encryption -- -D warnings
cargo fmt --all -- --check

# Run an example
cargo run --example basic_usage
```

## Reporting Issues

Use [GitHub Issues](https://github.com/LarsArtmann/segment-buffer/issues).

## License

By contributing, you agree that your contributions will be licensed under the
Apache License 2.0.
